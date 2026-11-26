//! Source-owned printer/process/material settings for project saves.
use super::paint_color::MAX_EXTRUDER_SLOT;
use super::presets::library;
use super::quantize::{quantize_face_colors, Quantized};
use crate::threemf::opc::Package;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

const PROJECT: &str = "Metadata/project_settings.config";
pub(crate) const GRADIENT_PRINT_PART: &str = "Metadata/cym/gradient_print.json";
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct GradientPrintMarker {
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) native_definitions_len: usize,
}
const MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone)]
pub struct SourcePresets {
    project: Value,
    files: Vec<(String, Vec<u8>)>,
    gradient_print: Option<usize>,
}

impl SourcePresets {
    pub fn read(path: &Path) -> Result<Option<Self>, String> {
        let mut package = Package::open_with_limit(path, MAX_SNAPSHOT_BYTES as u64)?;
        let names = package.part_names();
        if !names.iter().any(|name| name == PROJECT) {
            return Ok(None);
        }
        let bytes = package.read(PROJECT)?;
        let project: Value = serde_json::from_slice(&bytes)
            .map_err(|e| format!("Source project settings cannot be preserved: {e}"))?;
        if !project.is_object() {
            return Err("Source project settings must be an object".into());
        }
        let mut total = bytes.len();
        let mut files = Vec::new();
        for name in names.iter().filter(|name| is_preset_part(name)) {
            let body = package.read(name)?;
            total = total.saturating_add(body.len());
            if total > MAX_SNAPSHOT_BYTES {
                return Err("Source preset snapshot exceeds 32 MiB".into());
            }
            files.push((name.clone(), body));
        }
        let gradient_print = if names.iter().any(|name| name == GRADIENT_PRINT_PART) {
            serde_json::from_slice::<GradientPrintMarker>(&package.read(GRADIENT_PRINT_PART)?)
                .ok()
                .filter(|marker| matches!(marker.version, 1 | 2))
                .and_then(|marker| {
                    let defs = project["mixed_filament_definitions"].as_str().unwrap_or("");
                    let end = marker.native_definitions_len;
                    defs.get(..end).filter(|base| {
                        end == 0 || end == defs.len() || base.ends_with(';')
                            || defs.as_bytes().get(end) == Some(&b';')
                    }).map(|_| end)
                })
        } else {
            None
        };
        Ok(Some(Self { project, files, gradient_print }))
    }

    pub(crate) fn native_definitions(&self) -> &str {
        let definitions = self.project["mixed_filament_definitions"].as_str().unwrap_or("");
        self.gradient_print.map_or(definitions, |end| &definitions[..end])
    }

    pub(crate) fn native_slot_count(&self) -> Result<usize, String> {
        let source = serde_json::json!({
            "filament_colour": self.project["filament_colour"],
            "mixed_filament_definitions": self.native_definitions(),
        });
        Ok(crate::threemf::native_material_slot_count(&source)?
            .map_or(0, |(_, virtual_count)| virtual_count))
    }

    /// Native mixed recipes and the machine's process settings reference the
    /// original physical slot order.
    pub(crate) fn mixed_project_palette(
        &self,
        include_native: bool,
        has_gradient: bool,
    ) -> Result<Option<Vec<[u8; 3]>>, String> {
        let supports_mixing = !self.native_definitions().is_empty()
            || has_gradient && self.project["printer_model"].as_str()
                .is_some_and(|model| model.starts_with("Snapmaker"));
        if !(self.gradient_print.is_some() || include_native && supports_mixing) {
            return Ok(None);
        }
        let entries = self.project["filament_colour"].as_array()
            .filter(|colors| (1..=usize::from(MAX_EXTRUDER_SLOT)).contains(&colors.len()))
            .ok_or("Gradient print project has an invalid physical filament palette")?;
        entries.iter().map(parse_rgb).collect::<Result<Vec<_>, _>>().map(Some)
    }

    pub(crate) fn apply_unchanged(
        &self,
        entries: &mut Vec<(String, Vec<u8>)>,
    ) -> Result<(), String> {
        let project = serde_json::to_vec_pretty(&self.project).map_err(|error| error.to_string())?;
        entries.retain(|(name, _)| name != PROJECT && !is_preset_part(name));
        entries.push((PROJECT.into(), project));
        entries.extend(self.files.iter().cloned());
        Ok(())
    }

    pub fn quantize(&self, colors: &[[u8; 4]], face_count: usize) -> Result<Quantized, String> {
        let original = self.project["filament_colour"]
            .as_array()
            .ok_or("Source has no filament palette")?
            .iter()
            .map(parse_rgb)
            .collect::<Result<Vec<_>, _>>()?;
        let mut referenced = std::collections::BTreeSet::new();
        let mut configs = vec![self.project.clone()];
        for (name, bytes) in &self.files {
            if name.starts_with("Metadata/process_settings_") {
                configs
                    .push(serde_json::from_slice(bytes).map_err(|e| {
                        format!("Source process settings cannot be preserved: {e}")
                    })?);
            }
        }
        for cfg in configs {
            for key in SLOT_SELECTOR_KEYS {
                let Some(value) = cfg.get(*key) else {
                    continue;
                };
                let slot = value
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .or_else(|| value.as_str().and_then(|text| text.parse::<usize>().ok()))
                    .ok_or_else(|| format!("Source material selector {key} is invalid"))?;
                if slot > 0 {
                    if slot > original.len() {
                        return Err(format!(
                            "Source setting {key} references invalid material slot {slot}"
                        ));
                    }
                    referenced.insert(slot - 1);
                }
            }
        }
        let mut palette = Vec::new();
        for slot in referenced {
            let rgb = original[slot];
            if original.iter().filter(|&&color| color == rgb).count() > 1 {
                return Err("Referenced source materials have duplicate colours; use Export to choose materials explicitly".into());
            }
            palette.push(rgb);
        }
        let limit = MAX_EXTRUDER_SLOT as usize;
        if palette.len() > limit {
            return Err("More than 16 source materials are explicitly referenced".into());
        }
        let default = [[200, 200, 200, 255]];
        let source = if colors.is_empty() {
            &default[..]
        } else {
            colors
        };
        let budget = limit - palette.len();
        if budget > 0 {
            let unreserved: Vec<_> = source
                .iter()
                .filter(|c| !palette.contains(&[c[0], c[1], c[2]]))
                .copied()
                .collect();
            for rgb in quantize_face_colors(&unreserved, budget).palette {
                if !palette.contains(&rgb) {
                    palette.push(rgb);
                }
            }
        }
        if palette.is_empty() {
            palette.push([200, 200, 200]);
        }
        let nearest = |color: &[u8; 4]| -> u8 {
            palette
                .iter()
                .enumerate()
                .min_by_key(|(_, rgb)| {
                    (0..3)
                        .map(|i| {
                            let d = color[i] as i32 - rgb[i] as i32;
                            d * d
                        })
                        .sum::<i32>()
                })
                .unwrap()
                .0 as u8
                + 1
        };
        let face_slots = if colors.is_empty() {
            vec![nearest(&default[0]); face_count]
        } else {
            colors.iter().map(nearest).collect()
        };
        Ok(Quantized {
            palette,
            face_slots,
        })
    }

    pub fn apply(
        &self,
        palette: &[[u8; 3]],
        entries: &mut Vec<(String, Vec<u8>)>,
    ) -> Result<(), String> {
        let old = self.project["filament_colour"]
            .as_array()
            .ok_or("Source has no filament palette; choose explicit export settings")?;
        if old.is_empty() {
            return Err("Source filament palette is empty".into());
        }
        let original: Vec<[u8; 3]> = old.iter().map(parse_rgb).collect::<Result<_, _>>()?;
        let mapping: Vec<usize> = palette
            .iter()
            .map(|rgb| {
                original
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, candidate)| {
                        (0..3)
                            .map(|i| {
                                let d = rgb[i] as i32 - candidate[i] as i32;
                                d * d
                            })
                            .sum::<i32>()
                    })
                    .unwrap()
                    .0
            })
            .collect();
        let (mut filament_keys, mut other_keys) = setting_keys();
        let mut embedded = HashMap::new();
        for (name, bytes) in &self.files {
            let cfg: Value = serde_json::from_slice(bytes)
                .map_err(|e| format!("Source preset {name} cannot be preserved: {e}"))?;
            let obj = cfg
                .as_object()
                .ok_or_else(|| format!("Source preset {name} is not an object"))?;
            if let Some(slot) = filament_slot(name) {
                filament_keys.extend(obj.keys().cloned());
                embedded.insert(slot, cfg);
            } else {
                other_keys.extend(obj.keys().cloned());
            }
        }
        filament_keys.extend(["filament_settings_id", "filament_ids"].map(str::to_string));
        let mut project = self.project.clone();
        remap_slot_selectors(&mut project, &mapping)?;
        for (key, value) in project.as_object_mut().unwrap().iter_mut() {
            if key == "filament_colour" || key == "inherits_group" {
                continue;
            }
            if key == "flush_volumes_matrix" {
                let values = value
                    .as_array()
                    .ok_or("Source flushing matrix is not an array")?;
                if values.len() != old.len() * old.len() {
                    return Err("Source flushing matrix has invalid dimensions".into());
                }
                *value = Value::Array(
                    mapping
                        .iter()
                        .flat_map(|&from| {
                            mapping
                                .iter()
                                .map(move |&to| values[from * old.len() + to].clone())
                        })
                        .collect(),
                );
                continue;
            }
            if key == "flush_volumes_vector" {
                let values = value
                    .as_array()
                    .ok_or("Source flushing vector is not an array")?;
                if values.len() != old.len() * 2 {
                    return Err("Source flushing vector has invalid dimensions".into());
                }
                *value = Value::Array(
                    mapping
                        .iter()
                        .flat_map(|&i| values[i * 2..i * 2 + 2].iter().cloned())
                        .collect(),
                );
                continue;
            }
            let Some(values) = value.as_array() else {
                continue;
            };
            if (filament_keys.contains(key) || key.starts_with("filament_"))
                && !other_keys.contains(key)
            {
                if values.is_empty() {
                    continue;
                }
                if values.len() == 1 {
                    *value = Value::Array(mapping.iter().map(|_| values[0].clone()).collect());
                } else if values.len() % old.len() == 0 {
                    // Snapmaker stores standard/high-flow variants consecutively per material.
                    let width = values.len() / old.len();
                    *value = Value::Array(
                        mapping
                            .iter()
                            .flat_map(|&i| values[i * width..(i + 1) * width].iter().cloned())
                            .collect(),
                    );
                } else {
                    return Err(format!("Source material setting {key} has an unsupported slot count; use Export to choose settings"));
                }
            } else if !other_keys.contains(key) && values.len() == old.len() {
                return Err(format!("Cannot classify source setting {key}; use Export to choose settings explicitly"));
            }
        }
        if let Some(group) = self.project.get("inherits_group") {
            let group = group
                .as_array()
                .ok_or("Source inherits_group is not an array")?;
            if group.len() != old.len() + 2 {
                return Err("Source inherits_group has an unsupported slot count".into());
            }
            let mut mapped = vec![group[0].clone()];
            mapped.extend(mapping.iter().map(|&i| group[i + 1].clone()));
            mapped.push(group[group.len() - 1].clone());
            project["inherits_group"] = Value::Array(mapped);
        }
        project["filament_colour"] = Value::Array(
            palette
                .iter()
                .map(|c| Value::String(format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])))
                .collect(),
        );
        let mut replacement = vec![(
            PROJECT.into(),
            serde_json::to_vec_pretty(&project).map_err(|e| e.to_string())?,
        )];
        for (name, bytes) in self
            .files
            .iter()
            .filter(|(name, _)| filament_slot(name).is_none())
        {
            if name.starts_with("Metadata/process_settings_") {
                let mut cfg: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
                remap_slot_selectors(&mut cfg, &mapping)?;
                replacement.push((
                    name.clone(),
                    serde_json::to_vec_pretty(&cfg).map_err(|e| e.to_string())?,
                ));
            } else {
                replacement.push((name.clone(), bytes.clone()));
            }
        }
        for (i, &source_slot) in mapping.iter().enumerate() {
            if let Some(cfg) = embedded.get(&(source_slot + 1)) {
                let mut cfg = cfg.clone();
                cfg["filament_colour"] = Value::Array(vec![project["filament_colour"][i].clone()]);
                replacement.push((
                    format!("Metadata/filament_settings_{}.config", i + 1),
                    serde_json::to_vec_pretty(&cfg).map_err(|e| e.to_string())?,
                ));
            }
        }
        entries.retain(|(name, _)| name != PROJECT && !is_preset_part(name));
        entries.extend(replacement);
        Ok(())
    }
}

const SLOT_SELECTOR_KEYS: &[&str] = &[
    "extruder",
    "support_filament",
    "support_interface_filament",
    "wipe_tower_filament",
    "sparse_infill_filament",
    "wall_filament",
    "solid_infill_filament",
    "sparse_infill_filament_id",
    "outer_wall_filament_id",
    "inner_wall_filament_id",
    "internal_solid_filament_id",
    "top_surface_filament_id",
    "bottom_surface_filament_id",
    "outer_wall_filament",
    "inner_wall_filament",
    "wall_filament_id",
    "top_solid_infill_filament",
    "bottom_solid_infill_filament",
    "infill_extruder",
    "perimeter_extruder",
    "solid_infill_extruder",
    "support_material_extruder",
    "support_material_interface_extruder",
    "wipe_tower_extruder",
];

/// PrintConfig's feature-material selectors are 1-based; zero inherits defaults.
fn remap_slot_selectors(config: &mut Value, mapping: &[usize]) -> Result<(), String> {
    for key in SLOT_SELECTOR_KEYS {
        let Some(value) = config.get_mut(*key) else {
            continue;
        };
        let slot = match value {
            Value::String(text) => text.parse::<usize>().ok(),
            Value::Number(number) => number.as_u64().and_then(|n| usize::try_from(n).ok()),
            _ => None,
        }
        .ok_or_else(|| {
            format!("Source material selector {key} is invalid; use Export to choose settings")
        })?;
        if slot == 0 {
            continue;
        }
        let mapped = mapping.iter().position(|&old| old == slot - 1)
            .ok_or_else(|| format!("Source setting {key} references removed material slot {slot}; use Export to choose settings explicitly"))? + 1;
        *value = if value.is_string() {
            Value::String(mapped.to_string())
        } else {
            Value::from(mapped)
        };
    }
    Ok(())
}

fn is_preset_part(name: &str) -> bool {
    [
        "Metadata/machine_settings_",
        "Metadata/process_settings_",
        "Metadata/filament_settings_",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix) && name.ends_with(".config"))
}

fn filament_slot(name: &str) -> Option<usize> {
    name.strip_prefix("Metadata/filament_settings_")?
        .strip_suffix(".config")?
        .parse()
        .ok()
}

fn parse_rgb(value: &Value) -> Result<[u8; 3], String> {
    let text = value.as_str().unwrap_or("").trim_start_matches('#');
    if text.len() != 6 || !text.is_ascii() {
        return Err("Source palette contains an invalid RGB colour".into());
    }
    let mut rgb = [0; 3];
    for i in 0..3 {
        rgb[i] = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)
            .map_err(|_| "Source palette contains an invalid RGB colour")?;
    }
    Ok(rgb)
}

fn setting_keys() -> (HashSet<String>, HashSet<String>) {
    // Explicit categories from the slicer schema; array length alone is ambiguous.
    let schema: Value = serde_json::from_str(include_str!(
        "../../resources/presets/source_setting_keys.json"
    ))
    .expect("bundled setting categories");
    let category = |name: &str| {
        schema[name]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect::<HashSet<_>>()
    };
    let mut filament = category("filament");
    let mut other = category("print");
    other.extend(category("printer"));
    for machine in library() {
        for f in &machine.filaments {
            filament.extend(f.config.keys().cloned());
        }
        for p in &machine.processes {
            other.extend(p.config.keys().cloned());
        }
        for v in &machine.variants {
            other.extend(v.config.keys().cloned());
        }
    }
    other.extend(
        [
            "default_filament_profile",
            "print_compatible_printers",
            "compatible_printers",
            "nozzle_diameter",
            "nozzle_volume_type",
        ]
        .map(str::to_string),
    );
    (filament, other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn source() -> SourcePresets {
        SourcePresets {
            gradient_print: None,
            project: json!({"printer_settings_id":"Custom U1", "print_settings_id":"Fine Custom", "filament_colour":["#FF0000","#0000FF"], "filament_type":["PLA","PETG"], "nozzle_temperature":["210","240"], "filament_settings_id":["Red PLA","Blue PETG"], "nozzle_diameter":["0.4"], "inherits_group":["Fine","PLA","PETG","U1"], "my_scalar":"keep"}),
            files: vec![
                (
                    "Metadata/machine_settings_1.config".into(),
                    br#"{"printer_settings_id":"Custom U1","custom_gcode":"G28"}"#.to_vec(),
                ),
                (
                    "Metadata/process_settings_1.config".into(),
                    br#"{"layer_height":"0.12"}"#.to_vec(),
                ),
                (
                    "Metadata/filament_settings_1.config".into(),
                    br#"{"name":"Red PLA","filament_type":["PLA"]}"#.to_vec(),
                ),
                (
                    "Metadata/filament_settings_2.config".into(),
                    br#"{"name":"Blue PETG","filament_type":["PETG"]}"#.to_vec(),
                ),
            ],
        }
    }
    #[test]
    fn saving_preserves_source_printer_process_and_custom_settings_and_remaps_material_slots() {
        let mut entries = vec![
            (PROJECT.into(), b"{}".to_vec()),
            (
                "Metadata/machine_settings_1.config".into(),
                b"generic".to_vec(),
            ),
        ];
        source()
            .apply(&[[0, 0, 255], [255, 0, 0], [250, 0, 0]], &mut entries)
            .unwrap();
        let cfg: Value =
            serde_json::from_slice(&entries.iter().find(|(n, _)| n == PROJECT).unwrap().1).unwrap();
        assert_eq!(cfg["printer_settings_id"], "Custom U1");
        assert_eq!(cfg["print_settings_id"], "Fine Custom");
        assert_eq!(cfg["my_scalar"], "keep");
        assert_eq!(
            cfg["filament_colour"],
            json!(["#0000FF", "#FF0000", "#FA0000"])
        );
        assert_eq!(cfg["filament_type"], json!(["PETG", "PLA", "PLA"]));
        assert_eq!(cfg["nozzle_temperature"], json!(["240", "210", "210"]));
        assert_eq!(cfg["nozzle_diameter"], json!(["0.4"]));
        assert_eq!(
            cfg["inherits_group"],
            json!(["Fine", "PETG", "PLA", "PLA", "U1"])
        );
        let filament: Value = serde_json::from_slice(
            &entries
                .iter()
                .find(|(n, _)| n == "Metadata/filament_settings_1.config")
                .unwrap()
                .1,
        )
        .unwrap();
        assert_eq!(filament["name"], "Blue PETG");
        assert!(String::from_utf8_lossy(
            &entries
                .iter()
                .find(|(n, _)| n == "Metadata/machine_settings_1.config")
                .unwrap()
                .1
        )
        .contains("G28"));
    }
    #[test]
    fn unknown_slot_arrays_fail_without_mutating_output_entries() {
        let mut source = source();
        source.project["custom_unknown_array"] = json!(["a", "b"]);
        let mut entries = vec![(PROJECT.into(), b"original".to_vec())];
        assert!(source.apply(&[[0, 0, 255]], &mut entries).is_err());
        assert_eq!(entries[0].1, b"original");
    }
    #[test]
    fn saved_project_reopens_with_exact_paint_and_source_custom_presets() {
        let path = std::env::temp_dir().join(format!(
            "cym-source-preset-roundtrip-{}.3mf",
            std::process::id()
        ));
        let mut mesh = crate::mesh::model::MeshModel::new();
        mesh.vertices = vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
        mesh.faces = vec![[0, 1, 2]];
        mesh.face_colors = vec![[0, 0, 255, 255]];
        mesh.segment_labels = vec![0];
        mesh.source_presets = Some(Ok(source()));
        super::super::threemf::export_3mf_with_source(&mesh, &path, None, true, &|_, _| {})
            .unwrap();
        let mut package = Package::open(&path).unwrap();
        let cfg: Value = serde_json::from_slice(&package.read(PROJECT).unwrap()).unwrap();
        assert_eq!(cfg["printer_settings_id"], "Custom U1");
        assert_eq!(cfg["filament_type"], json!(["PETG"]));
        let restored = crate::threemf::read_project(&path, &|_, _| {}).unwrap();
        assert_eq!(restored.plates[0].mesh.face_colors, mesh.face_colors);
        assert_eq!(
            restored.plates[0].engineering_state,
            crate::threemf::EngineeringState::Restored
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn material_variants_and_flushing_dimensions_follow_remapped_slots() {
        let mut source = source();
        source.project["nozzle_temperature"] = json!(["210", "215", "240", "245"]);
        source.project["flush_volumes_matrix"] = json!(["0", "100", "200", "0"]);
        source.project["flush_volumes_vector"] = json!(["10", "20", "30", "40"]);
        let mut entries = Vec::new();
        source
            .apply(&[[0, 0, 255], [255, 0, 0]], &mut entries)
            .unwrap();
        let cfg: Value =
            serde_json::from_slice(&entries.iter().find(|(n, _)| n == PROJECT).unwrap().1).unwrap();
        assert_eq!(
            cfg["nozzle_temperature"],
            json!(["240", "245", "210", "215"])
        );
        assert_eq!(cfg["flush_volumes_matrix"], json!(["0", "200", "100", "0"]));
        assert_eq!(cfg["flush_volumes_vector"], json!(["30", "40", "10", "20"]));
    }

    #[test]
    fn project_and_embedded_process_slot_selectors_follow_palette_remapping() {
        let mut source = source();
        source.project["support_filament"] = Value::String("2".into());
        source.project["outer_wall_filament_id"] = json!(2);
        source.project["support_interface_filament"] = Value::String("0".into());
        source.files.iter_mut().find(|(name,_)| name == "Metadata/process_settings_1.config").unwrap().1 =
            br#"{"support_filament":"2","support_interface_filament":0,"wall_filament":"2","layer_height":"0.12"}"#.to_vec();
        let mut entries = Vec::new();
        source.apply(&[[0, 0, 255]], &mut entries).unwrap();
        let project: Value =
            serde_json::from_slice(&entries.iter().find(|(n, _)| n == PROJECT).unwrap().1).unwrap();
        assert_eq!(project["support_filament"], "1");
        assert_eq!(project["outer_wall_filament_id"], json!(1));
        assert_eq!(project["support_interface_filament"], "0");
        let process: Value = serde_json::from_slice(
            &entries
                .iter()
                .find(|(n, _)| n == "Metadata/process_settings_1.config")
                .unwrap()
                .1,
        )
        .unwrap();
        assert_eq!(process["support_filament"], "1");
        assert_eq!(process["wall_filament"], "1");
        assert_eq!(process["support_interface_filament"], json!(0));
    }

    #[test]
    fn a_removed_selector_material_rejects_save_without_changing_entries() {
        for embedded in [false, true] {
            let mut source = source();
            if embedded {
                source
                    .files
                    .iter_mut()
                    .find(|(name, _)| name == "Metadata/process_settings_1.config")
                    .unwrap()
                    .1 = br#"{"support_interface_filament":"1"}"#.to_vec();
            } else {
                source.project["support_filament"] = Value::String("1".into());
            }
            let mut entries = vec![(PROJECT.into(), b"original".to_vec())];
            let error = source.apply(&[[0, 0, 255]], &mut entries).unwrap_err();
            assert!(error.contains("removed material slot 1"), "{error}");
            assert_eq!(entries[0].1, b"original");
        }
    }

    #[test]
    fn saving_keeps_materials_referenced_only_by_process_settings() {
        let path =
            std::env::temp_dir().join(format!("cym-reserved-source-{}.3mf", std::process::id()));
        let mut source = source();
        source.project["support_filament"] = Value::String("1".into());
        source
            .files
            .iter_mut()
            .find(|(name, _)| name == "Metadata/process_settings_1.config")
            .unwrap()
            .1 = br#"{"support_interface_filament":"1"}"#.to_vec();
        let mut mesh = crate::mesh::model::MeshModel::new();
        mesh.vertices = vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
        mesh.faces = vec![[0, 1, 2]];
        mesh.face_colors = vec![[0, 0, 255, 255]];
        mesh.segment_labels = vec![0];
        mesh.source_presets = Some(Ok(source));
        super::super::threemf::export_3mf_with_source(&mesh, &path, None, true, &|_, _| {})
            .unwrap();
        let mut package = Package::open(&path).unwrap();
        let cfg: Value = serde_json::from_slice(&package.read(PROJECT).unwrap()).unwrap();
        let support: usize = cfg["support_filament"].as_str().unwrap().parse().unwrap();
        assert_eq!(cfg["filament_colour"][support - 1], "#FF0000");
        assert_eq!(cfg["filament_type"][support - 1], "PLA");
        assert_eq!(cfg["filament_colour"].as_array().unwrap().len(), 2);
        let restored = crate::threemf::read_project(&path, &|_, _| {}).unwrap();
        assert_eq!(restored.plates[0].mesh.face_colors, mesh.face_colors);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn sixteen_referenced_materials_leave_no_extra_palette_slots() {
        let mut source = source();
        source.files.clear();
        let original: Vec<[u8; 3]> = (0..16).map(|i| [i * 15, 0, 0]).collect();
        source.project = json!({"filament_colour": original.iter().map(|rgb| format!("#{:02X}0000",rgb[0])).collect::<Vec<_>>()});
        for (i, key) in SLOT_SELECTOR_KEYS.iter().take(16).enumerate() {
            source.project[*key] = Value::String((i + 1).to_string());
        }
        let quantized = source
            .quantize(&[[0, 255, 255, 255], [255, 255, 255, 255]], 2)
            .unwrap();
        assert_eq!(quantized.palette, original);
        assert_eq!(quantized.face_slots.len(), 2);
        assert!(quantized
            .face_slots
            .iter()
            .all(|&slot| (1..=16).contains(&slot)));
        let mut entries = Vec::new();
        source.apply(&quantized.palette, &mut entries).unwrap();
    }

    #[test]
    fn referenced_duplicate_source_colours_are_rejected_instead_of_switching_material() {
        let mut source = source();
        source.project["filament_colour"] = json!(["#FF0000", "#FF0000"]);
        source.project["support_filament"] = Value::String("2".into());
        assert!(source
            .quantize(&[[255, 0, 0, 255]], 1)
            .err()
            .unwrap()
            .contains("duplicate colours"));
    }

    #[test]
    #[ignore = "needs CYM_REAL_3MF"]
    fn real_source_settings_survive_palette_remapping() {
        let path = std::env::var("CYM_REAL_3MF").expect("fixture path");
        let source = SourcePresets::read(Path::new(&path)).unwrap().unwrap();
        let original_printer = source.project["printer_settings_id"].clone();
        let path = std::env::temp_dir().join(format!(
            "cym-real-partial-palette-{}.3mf",
            std::process::id()
        ));
        let mut mesh = crate::mesh::model::MeshModel::new();
        mesh.vertices = vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
        mesh.faces = vec![[0, 1, 2]];
        mesh.face_colors = vec![[0, 0, 255, 255]];
        mesh.segment_labels = vec![0];
        mesh.source_presets = Some(Ok(source.clone()));
        super::super::threemf::export_3mf_with_source(&mesh, &path, None, true, &|_, _| {})
            .unwrap();
        let mut package = Package::open(&path).unwrap();
        let cfg: Value = serde_json::from_slice(&package.read(PROJECT).unwrap()).unwrap();
        assert_eq!(cfg["printer_settings_id"], original_printer);
        for key in SLOT_SELECTOR_KEYS {
            if let Some(slot) = source
                .project
                .get(*key)
                .and_then(|v| v.as_str())
                .and_then(|text| text.parse::<usize>().ok())
            {
                if slot == 0 {
                    continue;
                }
                let remapped: usize = cfg[*key].as_str().unwrap().parse().unwrap();
                assert_eq!(
                    cfg["filament_colour"][remapped - 1],
                    source.project["filament_colour"][slot - 1]
                );
            }
        }
        assert!(cfg["filament_colour"].as_array().unwrap().len() <= 16);
        let restored = crate::threemf::read_project(&path, &|_, _| {}).unwrap();
        assert_eq!(restored.plates[0].mesh.face_colors, mesh.face_colors);
        std::fs::remove_file(path).unwrap();

        let mut entries = Vec::new();
        let palette: Vec<[u8; 3]> = source.project["filament_colour"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .map(parse_rgb)
            .collect::<Result<_, _>>()
            .unwrap();
        source.apply(&palette, &mut entries).unwrap();
        let project: Value =
            serde_json::from_slice(&entries.iter().find(|(n, _)| n == PROJECT).unwrap().1).unwrap();
        assert_eq!(project["printer_settings_id"], original_printer);
        assert_eq!(
            project["filament_colour"].as_array().unwrap().len(),
            palette.len()
        );
        assert_eq!(
            project["filament_type"].as_array().unwrap().len(),
            palette.len()
        );
    }
}
