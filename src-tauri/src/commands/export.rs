use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::commands::mesh::AppState;
use crate::commands::workspace::{SourcePlate, Workspace};
use crate::export::gradient::{ColorHistograms, GradientColors};
use crate::export::obj::export_obj;
use crate::export::paint_color::MAX_EXTRUDER_SLOT;
use crate::export::presets::{library, summary, ExportSelection, MachineSummary};
use crate::export::quantize::quantize_histogram;
use crate::export::source_presets::SourcePresets;
use crate::export::texture_mapping::{
    preview, suggestions, NominalColorPredictor, Rgb, TextureExportSnapshot, TextureMappingPreview,
    TextureMappingSuggestions, TextureSlotMapping,
};
use crate::export::threemf::{
    write_mapped_texture_3mf, write_project_3mf, ProjectPlate, ProjectWritePurpose,
};

/// The printer / process / filament catalogue the export dialog renders.
///
/// Returns the stripped summary rather than the full library: the flattened
/// preset configs are ~200 KB and the dialog only needs names, nozzle
/// compatibility and material labels.
#[tauri::command]
pub fn list_export_presets() -> Vec<MachineSummary> {
    summary()
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPalettePreview {
    flat: Vec<String>,
    gradient_by_slot_count: std::collections::BTreeMap<usize, GradientPalettePreview>,
}

#[derive(Debug, PartialEq, Serialize)]
struct GradientPalettePreview {
    colors: Vec<String>,
    approximated: bool,
}

/// The filament palettes the project would export, as `#RRGGBB`.
///
/// The export dialog renders one filament picker per entry. It has to come
/// from the backend: the dialog must show the slots that will actually be
/// written, and re-deriving them in TypeScript would mean two quantisers that
/// can drift apart.
#[tauri::command]
pub fn export_palette_preview(tab_id: u64, state: State<AppState>) -> Result<ExportPalettePreview, String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    ws.validate_active(tab_id)?;
    workspace_palette_preview(&mut ws, &mut |_| {})
}

#[tauri::command]
pub fn texture_mapping_preview(
    tab_id: u64,
    state: State<AppState>,
) -> Result<TextureMappingPreview, String> {
    let mesh = state.lock_mesh(tab_id)?;
    preview(mesh.as_ref().ok_or("No mesh loaded")?)
}

#[tauri::command]
pub fn suggest_texture_mapping(
    selection: ExportSelection,
    nominal_colors: Vec<Option<Rgb>>,
    colors: Vec<Rgb>,
) -> Result<TextureMappingSuggestions, String> {
    suggestions(&selection, &nominal_colors, &colors, &NominalColorPredictor)
}

#[derive(Deserialize)]
pub struct TextureMappingRequest {
    revision: String,
    mapping: TextureSlotMapping,
}

fn workspace_palette_preview(
    ws: &mut Workspace,
    observe: &mut dyn FnMut(usize),
) -> Result<ExportPalettePreview, String> {
    ws.ensure_idle()?;
    ws.evict_clean_inactive();
    let mut merged = std::collections::BTreeMap::new();
    let mut gradients = GradientColors::default();
    for index in 0..ws.tabs.len() {
        if ws.tabs[index].color_histogram.is_none() {
            ws.ensure_resident(index)?;
            observe(ws.tabs.iter().filter(|tab| tab.mesh.is_some()).count());
            let mesh = ws.tabs[index].mesh.as_ref().ok_or("No mesh loaded")?;
            ws.tabs[index].color_histogram = Some(ColorHistograms::from_mesh(mesh));
            ws.evict_clean_inactive();
        }
        let histograms = ws.tabs[index].color_histogram.as_ref().unwrap();
        for &(rgb, count) in &histograms.flat {
            *merged.entry(rgb).or_insert(0usize) += count;
        }
        gradients.absorb(histograms);
    }
    let histogram: Vec<_> = merged.into_iter().collect();
    let mut palette = quantize_histogram(&histogram, MAX_EXTRUDER_SLOT as usize);
    if palette.is_empty() && !ws.tabs.is_empty() {
        palette.push([200, 200, 200]);
    }
    let limits = library().iter()
        .filter(|machine| machine.model_ids.contains_key("snapmaker_orca"))
        .flat_map(|machine| machine.variants.iter().map(move |v| {
            machine.extruder_count(&v.nozzle_diameter).min(usize::from(MAX_EXTRUDER_SLOT))
        }))
        .collect::<std::collections::BTreeSet<_>>();
    let gradient_by_slot_count = limits.into_iter().filter_map(|limit| {
        let palette = gradients.palette(limit)?;
        Some((limit, GradientPalettePreview {
            colors: palette.colors.into_iter().map(hex).collect(),
            approximated: palette.approximated,
        }))
    }).collect();
    Ok(ExportPalettePreview {
        flat: palette.into_iter().map(hex).collect(),
        gradient_by_slot_count,
    })
}

fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// Build a progress callback that forwards to the frontend as `export-progress`
/// events. The command is `async`, so it runs off the webview thread and these
/// events arrive live instead of being batched until the call returns.
fn export_progress(app: &AppHandle) -> impl Fn(f32, &str) {
    let app = app.clone();
    move |frac: f32, stage: &str| {
        let _ = app.emit(
            "export-progress",
            serde_json::json!({ "progress": frac, "stage": stage }),
        );
    }
}

/// Preserve the exact path chosen in the native dialog or current project.
/// Changing a confirmed filename could silently overwrite a different file.
fn confirmed_output_path(path: &str) -> Result<PathBuf, String> {
    let output = PathBuf::from(path);
    if output.file_name().is_none() {
        return Err("Output path has no filename".into());
    }
    Ok(output)
}

/// Successful whole-project writes become the lazy source for the new plate order.
fn save_workspace(
    ws: &mut Workspace,
    path: &std::path::Path,
    selection: Option<&ExportSelection>,
    preserve_source: bool,
    progress: &dyn Fn(f32, &str),
) -> Result<usize, String> {
    let result = (|| {
        if ws.project_path.is_none() {
            if let Ok(destination) = path.canonicalize() {
                if ws
                    .tabs
                    .iter()
                    .filter_map(|tab| tab.source.as_ref())
                    .any(|source| {
                        source
                            .path
                            .canonicalize()
                            .is_ok_and(|original| original == destination)
                    })
                {
                    return Err("Choose a different path to keep the imported 3MF intact".into());
                }
            }
        }
        ws.ensure_all_resident()?;
        if !preserve_source
            && ws
                .tabs
                .iter()
                .any(|tab| tab.mesh.as_ref().is_some_and(|mesh| mesh.texture_painting))
        {
            return Err("Export each texture plate individually to confirm its filament slots. Use Save Project to save all plates together.".into());
        }
        let active = ws.index(ws.active.ok_or("No plate is open")?)?;
        let source = if preserve_source {
            ws.source_presets
                .as_ref()
                .map(|s| s.as_ref().map_err(Clone::clone))
                .transpose()?
        } else {
            None
        };
        let plates: Vec<_> = ws
            .tabs
            .iter()
            .map(|tab| ProjectPlate {
                name: tab.name.clone(),
                mesh: tab.mesh.as_ref().unwrap(),
            })
            .collect();
        let purpose = if preserve_source {
            ProjectWritePurpose::EngineeringSave
        } else {
            ProjectWritePurpose::PrintExport
        };
        let report =
            write_project_3mf(&plates, active, path, selection, source, purpose, progress)?;
        let final_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        ws.source_presets = match SourcePresets::read(&final_path) {
            Ok(Some(source)) => Some(Ok(source)),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        };
        ws.project_path = Some(final_path.clone());
        ws.source_layout = None;
        let count = ws.tabs.len() as u32;
        for (index, tab) in ws.tabs.iter_mut().enumerate() {
            tab.source = Some(SourcePlate {
                path: final_path.clone(),
                index,
                count,
            });
            tab.dirty = false;
            tab.engineering_state = Some("restored".into());
            tab.history_dropped = report.history_truncated[index];
            if let Some(mesh) = tab.mesh.as_mut() {
                mesh.source_path = Some(final_path.clone());
                mesh.source_plate_count = count;
                mesh.history.set_persisted_truncated(tab.history_dropped);
            }
        }
        Ok(report.omitted_gradient_recipes)
    })();
    ws.evict_clean_inactive();
    result
}

fn print_texture_plate(
    ws: &mut Workspace,
    tab_id: u64,
    path: &Path,
    texture: &TextureMappingRequest,
    progress: &dyn Fn(f32, &str),
) -> Result<(), String> {
    let active = ws.validate_active(tab_id)?;
    if !path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("3mf"))
    {
        return Err("Choose a .3mf filename for the texture plate".into());
    }
    let destination = path.canonicalize().ok();
    let protected = |source: &Path| {
        source == path
            || destination.as_ref().is_some_and(|destination| {
                source
                    .canonicalize()
                    .is_ok_and(|source| &source == destination)
            })
    };
    if ws.project_path.as_deref().is_some_and(protected)
        || ws
            .tabs
            .iter()
            .filter_map(|tab| tab.source.as_ref())
            .any(|source| protected(&source.path))
    {
        return Err("Choose a separate destination for the active plate's print export".into());
    }

    let tab = &mut ws.tabs[active];
    let mesh = tab.mesh.as_mut().ok_or("No mesh loaded")?;
    let snapshot = TextureExportSnapshot::build(mesh, &texture.revision, &texture.mapping)?;
    write_mapped_texture_3mf(
        &ProjectPlate {
            name: tab.name.clone(),
            mesh,
        },
        path,
        &snapshot,
        progress,
    )?;
    if mesh.texture_slot_mapping.as_ref() != Some(&snapshot.mapping) || mesh.texture_state_dropped {
        mesh.texture_slot_mapping = Some(snapshot.mapping);
        mesh.texture_state_dropped = false;
        tab.dirty = true;
    }
    Ok(())
}

// Older callers parse "Exported to <path>". Only a caller requesting the
// save report receives a structured warning when a native recipe cannot fit.
#[derive(serde::Serialize)]
#[serde(untagged)]
pub enum ExportCompletion {
    Message(String),
    SaveWarning {
        message: String,
        #[serde(rename = "omittedGradientRecipes")]
        omitted_gradient_recipes: usize,
    },
}

/// Export the loaded mesh as a `.3mf`.
///
/// `selection` is optional. `None` keeps the original behaviour — a synthetic
/// "Generic Printer / 0.20mm Standard / Generic PLA" profile — so the command
/// still works before the user has ever opened the export dialog, and so a
/// broken preset library can never block an export outright.
#[tauri::command]
pub async fn export_3mf_command(
    tab_id: u64,
    path: String,
    selection: Option<ExportSelection>,
    preserve_source: Option<bool>,
    report_warnings: Option<bool>,
    texture: Option<TextureMappingRequest>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ExportCompletion, String> {
    let mut workspace = state.workspace.lock().map_err(|e| e.to_string())?;
    workspace.validate_active(tab_id)?;
    let output_path = confirmed_output_path(&path)?;
    let progress = export_progress(&app);
    progress(0.05, "3mf:start");
    let preserve_source = preserve_source.unwrap_or(false);
    let omitted_gradient_recipes = if let Some(texture) = texture.as_ref() {
        if preserve_source {
            return Err(
                "Save the workspace separately from the mapped texture plate export".into(),
            );
        }
        print_texture_plate(&mut workspace, tab_id, &output_path, texture, &progress)?;
        0
    } else {
        save_workspace(
            &mut workspace,
            &output_path,
            selection.as_ref(),
            preserve_source,
            &progress,
        )?
    };
    progress(0.97, "3mf:verify");

    // Adversarial hardening: do NOT trust zip.finish()'s Ok. Confirm the file
    // actually landed on disk and is non-empty, otherwise surface the real
    // failure instead of leaving the user with "100% progress, no file".
    let meta = std::fs::metadata(&output_path)
        .map_err(|e| format!("3MF export finished but file missing: {}", e))?;
    if meta.len() == 0 {
        return Err("3MF export finished but the file is empty (0 bytes)".into());
    }
    let final_path = output_path
        .canonicalize()
        .unwrap_or_else(|_| output_path.clone());
    progress(1.0, "done");

    let message = format!("Exported to {}", final_path.display());
    if omitted_gradient_recipes > 0 && report_warnings == Some(true) {
        Ok(ExportCompletion::SaveWarning { message, omitted_gradient_recipes })
    } else {
        Ok(ExportCompletion::Message(message))
    }
}

/// Export the loaded mesh as a `.obj` + sibling `.mtl`, carrying per-face RGB
/// colour through `usemtl` groups. Unlike 3MF, OBJ needs no machine / process /
/// filament selection — the colour is written directly, so the export dialog
/// skips those pickers for this format.
#[tauri::command]
pub async fn export_obj_command(
    tab_id: u64,
    path: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let mesh_guard = state.lock_mesh(tab_id)?;
    let mesh = mesh_guard.as_ref().ok_or("No mesh loaded")?;

    let output_path = confirmed_output_path(&path)?;

    let progress = export_progress(&app);
    progress(0.05, "obj:start");
    export_obj(mesh, &output_path, &progress)?;
    progress(1.0, "done");

    let mtl_name = output_path
        .with_extension("mtl")
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("model.mtl")
        .to_string();
    Ok(format!(
        "Exported to {} (with {})",
        output_path.display(),
        mtl_name
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn textured_mesh(color: [u8; 4], slot: u8) -> crate::mesh::model::MeshModel {
        use crate::export::texture_mapping::TextureColorAssignment;
        let mut mesh = crate::mesh::model::MeshModel::new();
        mesh.vertices = vec![[30.0, 30.0, 0.0], [40.0, 30.0, 0.0], [30.0, 40.0, 0.0]];
        mesh.faces = vec![[0, 1, 2]];
        mesh.face_colors = vec![[11, 12, 13, 255]];
        mesh.segment_labels = vec![0];
        mesh.texture_painting = true;
        mesh.apply_paint(None, &[(0, color)]);
        mesh.texture_slot_mapping = Some(TextureSlotMapping {
            selection: ExportSelection {
                machine_id: "snapmaker_u1".into(),
                nozzle_diameter: "0.4".into(),
                process_name: "0.20 Standard @Snapmaker U1 (0.4 nozzle)".into(),
                filament_names: vec!["Generic PLA".into(); 4],
                target_slicer: "snapmaker_orca".into(),
            },
            nominal_colors: vec![Some([5, 6, 7]), None, None, Some([50, 60, 70])],
            assignments: vec![TextureColorAssignment {
                rgb: [color[0], color[1], color[2]],
                slot,
            }],
        });
        mesh
    }

    #[test]
    fn mapped_print_keeps_the_workspace_and_accepts_only_its_active_plate() {
        let dir = tempfile::tempdir().unwrap();
        let saved = dir.path().join("whole.3mf");
        let printed = dir.path().join("active.3mf");
        let mut workspace = Workspace::new();
        let first = workspace
            .append(textured_mesh([70, 80, 90, 255], 2), "one".into())
            .unwrap();
        let second = workspace
            .append(textured_mesh([40, 50, 60, 255], 3), "two".into())
            .unwrap();
        save_workspace(&mut workspace, &saved, None, true, &|_, _| {}).unwrap();
        assert!(workspace.tabs[0].mesh.is_none());
        let original_archive = std::fs::read(&saved).unwrap();
        let mesh = workspace.tabs[1].mesh.as_ref().unwrap();
        let mut mapping = mesh.texture_slot_mapping.as_ref().unwrap().clone();
        mapping.assignments[0].slot = 4;
        let request = TextureMappingRequest {
            revision: preview(mesh).unwrap().revision,
            mapping,
        };
        assert!(
            print_texture_plate(&mut workspace, first, &printed, &request, &|_, _| {}).is_err()
        );
        assert!(print_texture_plate(&mut workspace, second, &saved, &request, &|_, _| {}).is_err());
        assert!(save_workspace(&mut workspace, &printed, None, false, &|_, _| {}).is_err());
        assert!(!printed.exists());
        print_texture_plate(&mut workspace, second, &printed, &request, &|_, _| {}).unwrap();
        assert_eq!(
            workspace.project_path.as_deref(),
            Some(saved.canonicalize().unwrap().as_path())
        );
        assert_eq!(std::fs::read(&saved).unwrap(), original_archive);
        assert_eq!(
            workspace
                .infos()
                .iter()
                .map(|tab| tab.dirty)
                .collect::<Vec<_>>(),
            [false, true]
        );
        let output = crate::threemf::read_project(&printed, &|_, _| {}).unwrap();
        assert_eq!(output.plates.len(), 1);
        let mesh = &output.plates[0].mesh;
        assert_eq!(mesh.face_colors, [[40, 50, 60, 255]]);
        assert_eq!(
            mesh.texture_slot_mapping.as_ref().unwrap().assignments[0].slot,
            4
        );

        let mut zip = zip::ZipArchive::new(std::fs::File::open(&printed).unwrap()).unwrap();
        let settings: serde_json::Value =
            serde_json::from_reader(zip.by_name("Metadata/project_settings.config").unwrap())
                .unwrap();
        assert_eq!(settings["filament_colour"][3], "#323C46");
        let mut model = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("3D/3dmodel.model").unwrap(), &mut model)
            .unwrap();
        assert!(model.contains("paint_color=\"1C\""));
    }

    #[test]
    fn saved_texture_mappings_and_history_are_isolated_across_lazy_plate_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let saved = dir.path().join("textured-plates.3mf");
        let mut workspace = Workspace::new();
        let colors = [[70, 80, 90, 255], [40, 50, 60, 255]];
        workspace
            .append(textured_mesh(colors[0], 2), "one".into())
            .unwrap();
        workspace
            .append(textured_mesh(colors[1], 4), "two".into())
            .unwrap();
        save_workspace(&mut workspace, &saved, None, true, &|_, _| {}).unwrap();

        let mut reopened = Workspace::new();
        reopened.open_project(&saved, &|_, _| {}).unwrap();
        assert!(reopened.tabs[0].mesh.is_none());
        for (index, expected_slot) in [2, 4].into_iter().enumerate() {
            let tab = reopened.tabs[index].id;
            reopened.switch(tab).unwrap();
            let mesh = reopened.tabs[index].mesh.as_mut().unwrap();
            assert!(mesh.texture_painting);
            assert!(!mesh.texture_state_dropped);
            assert!(mesh.segments.is_empty());
            assert_eq!(mesh.face_colors, [colors[index]]);
            assert_eq!(
                mesh.texture_slot_mapping.as_ref().unwrap().assignments[0].slot,
                expected_slot
            );
            mesh.history
                .undo(&mut mesh.face_colors, &mut mesh.segment_labels)
                .unwrap();
            assert_eq!(mesh.face_colors, [[11, 12, 13, 255]]);
        }
    }

    #[test]
    fn confirmed_filenames_are_preserved_including_repeated_suffixes_and_case() {
        for path in [
            "model.3mf.3mf",
            "model.3MF",
            "model.obj.obj",
            "model.OBJ",
            "model",
        ] {
            assert_eq!(confirmed_output_path(path).unwrap(), PathBuf::from(path));
        }
    }

    #[test]
    fn first_cym_save_keeps_an_external_3mf_intact() {
        use crate::threemf::test_support::{mesh_object, model_xml, Fixture};

        let vertices = [[30.0, 30.0, 0.0], [40.0, 30.0, 0.0], [30.0, 40.0, 0.0]];
        let object = mesh_object(1, "Source", &vertices, &[[0, 1, 2]], &[None]);
        let fixture = Fixture::new(&[(
            "3D/3dmodel.model",
            model_xml("millimeter", &object, r#"<item objectid="1"/>"#),
        )]);
        let input = fixture.path();
        let original = std::fs::read(input).unwrap();
        let mut ws = Workspace::new();
        ws.open_project(input, &|_, _| {}).unwrap();
        assert!(ws.project_path.is_none());
        let error = save_workspace(&mut ws, input, None, true, &|_, _| {}).unwrap_err();
        assert!(error.contains("different path"));

        let output = input.with_extension("cym.3mf");
        save_workspace(&mut ws, &output, None, true, &|_, _| {}).unwrap();
        assert_eq!(std::fs::read(input).unwrap(), original);
        assert_eq!(ws.project_path, Some(output.canonicalize().unwrap()));
        save_workspace(&mut ws, &output, None, true, &|_, _| {}).unwrap();
        std::fs::remove_file(output).unwrap();
    }

    #[test]
    fn repeated_suffix_exports_do_not_overwrite_the_shorter_neighbor() {
        let root =
            std::env::temp_dir().join(format!("cym-exact-export-path-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut ws = Workspace::new();
        let mut mesh = crate::mesh::model::MeshModel::new();
        mesh.vertices = vec![[30.0, 30.0, 0.0], [40.0, 30.0, 0.0], [30.0, 40.0, 0.0]];
        mesh.faces = vec![[0, 1, 2]];
        mesh.face_colors = vec![[255, 0, 0, 255]];
        mesh.segment_labels = vec![0];
        ws.append(mesh, "plate".into()).unwrap();
        for (ext, name) in [("3mf", "model.3mf.3mf"), ("obj", "model.obj.obj")] {
            let neighbor = root.join(format!("model.{ext}"));
            std::fs::write(&neighbor, b"unrelated existing model").unwrap();
            let chosen = root.join(name);
            let output = confirmed_output_path(chosen.to_str().unwrap()).unwrap();
            if ext == "3mf" {
                save_workspace(&mut ws, &output, None, false, &|_, _| {}).unwrap();
            } else {
                export_obj(ws.tabs[0].mesh.as_ref().unwrap(), &output, &|_, _| {}).unwrap();
            }
            assert_eq!(
                std::fs::read(&neighbor).unwrap(),
                b"unrelated existing model"
            );
            assert!(chosen.is_file());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    fn oversize_mesh(color: [u8; 4]) -> crate::mesh::model::MeshModel {
        let mut mesh = crate::mesh::model::MeshModel::new();
        mesh.vertices = vec![[0.0, 0.0, 0.0], [300.0, 0.0, 0.0], [0.0, 300.0, 0.0]];
        mesh.faces = vec![[0, 1, 2]];
        mesh.face_colors = vec![[20, 30, 40, 255]];
        mesh.segment_labels = vec![100001];
        mesh.segment_names.insert(100001, "Large region".into());
        mesh.next_manual_label = 100002;
        mesh.apply_paint(Some(1), &[(0, color)]);
        mesh
    }

    #[test]
    fn engineering_save_of_a_300mm_stl_preserves_geometry_and_undo() {
        let root = std::env::temp_dir().join(format!("cym-oversize-save-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("large-project.3mf");
        let mut ws = Workspace::new();
        let mesh = oversize_mesh([255, 0, 0, 255]);
        let vertices = mesh.vertices.clone();
        ws.append(mesh, "Large STL".into()).unwrap();
        save_workspace(&mut ws, &path, None, true, &|_, _| {}).unwrap();
        let mut loaded = crate::threemf::load_plate(&path, 0, &|_, _| {}).unwrap();
        assert_eq!(loaded.mesh.vertices, vertices);
        assert_eq!(loaded.mesh.face_colors, vec![[255, 0, 0, 255]]);
        assert_eq!(
            loaded.engineering_state,
            crate::threemf::EngineeringState::Restored
        );
        loaded
            .mesh
            .history
            .undo(
                &mut loaded.mesh.face_colors,
                &mut loaded.mesh.segment_labels,
            )
            .unwrap();
        assert_eq!(loaded.mesh.face_colors, vec![[20, 30, 40, 255]]);
        // Saving work must not silently enlarge the actual printer bed.
        let mut package = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        let settings: serde_json::Value =
            serde_json::from_reader(package.by_name("Metadata/project_settings.config").unwrap())
                .unwrap();
        assert_eq!(
            settings["printable_area"],
            serde_json::json!(["0x0", "220x0", "220x220", "0x220"])
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn oversize_multi_plate_save_round_trips_but_print_export_remains_strict() {
        let root =
            std::env::temp_dir().join(format!("cym-oversize-multi-save-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("large-project.3mf");
        let mut ws = Workspace::new();
        let colors = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]];
        for (index, color) in colors.iter().enumerate() {
            ws.append(oversize_mesh(*color), format!("Large {index}"))
                .unwrap();
        }
        let active = ws.tabs[1].id;
        ws.switch(active).unwrap();
        save_workspace(&mut ws, &path, None, true, &|_, _| {}).unwrap();
        let index = crate::threemf::load_plates(&path).unwrap();
        assert_eq!(index.plates.len(), 3);
        assert_eq!(index.active_index, 1);
        for (i, color) in colors.iter().enumerate() {
            let mut plate = crate::threemf::load_plate(&path, i, &|_, _| {}).unwrap();
            assert_eq!(plate.name, format!("Large {i}"));
            assert_eq!(
                plate.mesh.vertices,
                vec![[0.0, 0.0, 0.0], [300.0, 0.0, 0.0], [0.0, 300.0, 0.0]]
            );
            assert_eq!(plate.mesh.face_colors, vec![*color]);
            assert_eq!(plate.mesh.segment_labels, vec![100001]);
            assert_eq!(plate.mesh.segment_names[&100001], "Large region");
            assert_eq!(
                plate.engineering_state,
                crate::threemf::EngineeringState::Restored
            );
            plate
                .mesh
                .history
                .undo(&mut plate.mesh.face_colors, &mut plate.mesh.segment_labels)
                .unwrap();
            assert_eq!(plate.mesh.face_colors, vec![[20, 30, 40, 255]]);
        }
        let print_path = root.join("print.3mf");
        let error = save_workspace(&mut ws, &print_path, None, false, &|_, _| {}).unwrap_err();
        assert!(error.contains("does not fit"));
        assert!(!print_path.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn whole_project_save_retargets_closed_and_reordered_source_plates() {
        let root =
            std::env::temp_dir().join(format!("cym-full-workspace-save-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("project.3mf");
        let mut ws = Workspace::new();
        for i in 0..3 {
            let mut mesh = crate::mesh::model::MeshModel::new();
            mesh.vertices = vec![[30.0, 30.0, 0.0], [40.0, 30.0, 0.0], [30.0, 40.0, 0.0]];
            mesh.faces = vec![[0, 1, 2]];
            mesh.face_colors = vec![[i * 80, 2, 3, 255]];
            mesh.segment_labels = vec![0];
            ws.append(mesh, format!("plate {i}")).unwrap();
        }
        save_workspace(&mut ws, &path, None, false, &|_, _| {}).unwrap();
        assert_eq!(ws.tabs.len(), 3);
        assert!(!ws.tabs.iter().any(|t| t.dirty));
        let removed = ws.tabs[0].id;
        ws.close(removed).unwrap();
        save_workspace(&mut ws, &path, None, true, &|_, _| {}).unwrap();
        assert_eq!(ws.tabs.len(), 2);
        let first = ws.tabs[0].id;
        ws.switch(first).unwrap();
        assert_eq!(
            ws.tabs[0].mesh.as_ref().unwrap().face_colors[0],
            [80, 2, 3, 255]
        );
        let project = crate::threemf::read_project(&path, &|_, _| {}).unwrap();
        assert_eq!(project.plates.len(), 2);
        assert_eq!(project.plates[0].name, "plate 1");
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    #[test]
    fn preview_visits_at_most_one_clean_inactive_mesh_and_reuses_cached_histograms() {
        let root =
            std::env::temp_dir().join(format!("cym-preview-histogram-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("project.3mf");
        let mut ws = Workspace::new();
        for i in 0..3 {
            let mut mesh = crate::mesh::model::MeshModel::new();
            mesh.vertices = vec![[30.0, 30.0, 0.0], [40.0, 30.0, 0.0], [30.0, 40.0, 0.0]];
            mesh.faces = vec![[0, 1, 2]];
            mesh.face_colors = vec![[i * 80, 2, 3, 255]];
            mesh.segment_labels = vec![0];
            ws.append(mesh, format!("plate {i}")).unwrap();
        }
        save_workspace(&mut ws, &path, None, false, &|_, _| {}).unwrap();
        let mut peak = 0;
        let first =
            workspace_palette_preview(&mut ws, &mut |resident| peak = peak.max(resident)).unwrap();
        assert!(
            peak <= 2,
            "must retain active plus at most one clean inactive plate"
        );
        assert_eq!(first.flat.len(), 3);
        assert!(ws.tabs.iter().all(|t| t.color_histogram.is_some()));
        std::fs::remove_file(&path).unwrap();
        let again = workspace_palette_preview(&mut ws, &mut |_| {}).unwrap();
        assert_eq!(
            again, first,
            "cached preview must not reread inactive source files"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod histogram_invalidation_tests {
    use super::*;
    #[test]
    fn only_committed_edits_discard_the_cached_preview_histogram() {
        let state = AppState::new();
        let id = state
            .workspace
            .lock()
            .unwrap()
            .append(crate::mesh::model::MeshModel::new(), "A".into())
            .unwrap();
        state.workspace.lock().unwrap().tabs[0].color_histogram = Some(ColorHistograms {
            flat: vec![([1, 2, 3], 1)],
            gradient: None,
        });
        state.lock_mesh(id).unwrap().commit_if(false);
        assert!(state.workspace.lock().unwrap().tabs[0]
            .color_histogram
            .is_some());
        state.lock_mesh(id).unwrap().commit_if(true);
        assert!(state.workspace.lock().unwrap().tabs[0]
            .color_histogram
            .is_none());
    }
}

#[cfg(test)]
mod failed_save_tests {
    use super::*;
    use crate::mesh::model::MeshModel;

    fn workspace_with_dirty_and_lazy_plates(topic: &str) -> (Workspace, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("cym-failed-save-{topic}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("source.3mf");
        let mut ws = Workspace::new();
        for i in 0..4 {
            let mut mesh = MeshModel::new();
            mesh.vertices = vec![[30.0, 30.0, 0.0], [40.0, 30.0, 0.0], [30.0, 40.0, 0.0]];
            mesh.faces = vec![[0, 1, 2]];
            mesh.face_colors = vec![[i * 60, 2, 3, 255]];
            mesh.segment_labels = vec![0];
            ws.append(mesh, format!("plate {i}")).unwrap();
        }
        save_workspace(&mut ws, &path, None, false, &|_, _| {}).unwrap();
        ws.switch(ws.tabs[0].id).unwrap();
        ws.tabs[0]
            .mesh
            .as_mut()
            .unwrap()
            .apply_paint(None, &[(0, [7, 8, 9, 255])]);
        ws.tabs[0].dirty = true;
        ws.switch(ws.tabs[3].id).unwrap();
        assert!(ws.tabs[1].mesh.is_none());
        assert!(ws.tabs[2].mesh.is_none());
        (ws, root)
    }

    fn assert_failed_save_kept_edits_and_released_clean_plates(
        ws: &Workspace,
        active: Option<u64>,
    ) {
        assert_eq!(ws.active, active);
        assert!(ws.tabs[0].dirty);
        let dirty = ws.tabs[0].mesh.as_ref().unwrap();
        assert_eq!(dirty.face_colors[0], [7, 8, 9, 255]);
        assert!(dirty.history.can_undo());
        assert_eq!(
            ws.tabs[3].mesh.as_ref().unwrap().face_colors[0],
            [180, 2, 3, 255]
        );
        assert!(!ws.tabs[3].dirty);
        assert!(
            ws.tabs[1].mesh.is_none(),
            "temporary clean plate must be released"
        );
        assert!(ws.tabs[2].mesh.is_none());
    }

    #[test]
    fn source_preset_failure_releases_temporary_clean_plates() {
        let (mut ws, root) = workspace_with_dirty_and_lazy_plates("presets");
        let active = ws.active;
        let original_path = ws.project_path.clone();
        let source_bytes = std::fs::read(root.join("source.3mf")).unwrap();
        ws.source_presets = Some(Err("Source project settings cannot be preserved".into()));
        let output = root.join("output.3mf");
        let error = save_workspace(&mut ws, &output, None, true, &|_, _| {}).unwrap_err();
        assert!(error.contains("Source project settings"));
        assert!(!output.exists());
        assert_eq!(ws.project_path, original_path);
        assert_eq!(
            std::fs::read(root.join("source.3mf")).unwrap(),
            source_bytes
        );
        assert_failed_save_kept_edits_and_released_clean_plates(&ws, active);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn partial_source_load_failure_releases_previously_loaded_clean_plates() {
        let (mut ws, root) = workspace_with_dirty_and_lazy_plates("load");
        let active = ws.active;
        let original_path = ws.project_path.clone();
        ws.tabs[2].source.as_mut().unwrap().path = root.join("missing-source.3mf");
        let output = root.join("output.3mf");
        assert!(save_workspace(&mut ws, &output, None, true, &|_, _| {}).is_err());
        assert!(!output.exists());
        assert_eq!(ws.project_path, original_path);
        assert_failed_save_kept_edits_and_released_clean_plates(&ws, active);
        std::fs::remove_dir_all(root).unwrap();
    }
}
