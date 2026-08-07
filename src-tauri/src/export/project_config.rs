//! Writers for the JSON blobs OrcaSlicer reads when loading an exported `.3mf`.
//!
//! Files emitted:
//! - `Metadata/project_settings.config` — the project-level "what printer,
//!   what process, what colour map" snapshot.
//! - `Metadata/machine_settings_1.config` — embedded printer profile, **only
//!   in the "user picked a machine" path**.
//! - `Metadata/process_settings_1.config` — embedded process profile, **only
//!   in the "user picked a machine" path**.
//!
//! ## Why the project file is one flat config (REFUTE-5)
//!
//! `PresetBundle::load_config_file_config` (`PresetBundle.cpp:2779`) loads this
//! file as ONE `DynamicPrintConfig` holding all three preset categories, not as
//! a manifest pointing at the embedded files:
//!
//! - Slot count comes from `filament_colour.size()` (`PresetBundle.cpp:2779`).
//!   The comment right above it says `filament_settings_id` is deliberately not
//!   used for that because it "sometimes is not generated".
//! - For >1 slots the loader scatters every vector option by slot index:
//!   `configs[i].option(key)->set_at(other_opt, 0, i)` (`PresetBundle.cpp:2925`).
//!   Mixed materials therefore require N-length arrays here.
//! - `inherits_group` is `num_filaments + 2` entries: `[0]` process parent,
//!   `[1..=N]` filament parents, `[N+1]` printer parent
//!   (`PresetBundle.cpp:2861/2876/2943`). A parent name that resolves to
//!   nothing degrades the preset to defaults (`Preset.cpp:1541-1545`), so the
//!   printer entry should point at the machine's real `inherits` target
//!   (`fdm_U1`) when known.
//!
//! ## Why embedded machine/process files exist in the selected path (REFUTE-6)
//!
//! The dropdown label comes from the *selected preset*, and for system presets
//! Orca rewrites the label to `printer_model` ("Snapmaker U1" — the
//! parenthesised nozzle part is dropped, `PresetComboBoxes.cpp:1352-1356`).
//! Only a **project-embedded / user preset** keeps its full name
//! (`PresetComboBoxes.cpp:1373-1380`), so to make the slicer show
//! "Snapmaker U1 (0.4 nozzle)" we must ship `machine_settings_1.config` with
//! `printer_settings_id` exactly equal to the project file's
//! `printer_settings_id`. The loader first registers embedded presets
//! (`Plater.cpp:10955`) then selects by name from the project file
//! (`Plater.cpp:11079`); the names must match **character for character** or a
//! new "…(yourfile.3mf)" preset is synthesised.
//!
//! REFUTE-6 also showed the embedded machine preset must be a **complete**
//! config: `Preset.cpp:1541-1545` falls back to the default printer's values
//! for anything missing. The vendor presets extracted by
//! `tools/extract_orca_presets.py` carry the full flattened config (gcode
//! macros included), which is what we emit.
//!
//! ## Why no embedded filament files (REFUTE-7)
//!
//! An earlier revision wrote `filament_settings_<n>.config` per slot with
//! uniquified names (`Generic PLA @slot1..N`). Both halves were wrong:
//!
//! - Colours never come from filament presets. The swatch reads
//!   `project_config.filament_colour[extruder_idx]`
//!   (`PresetComboBoxes.cpp:1273`, `Plater.cpp:6630`); `Plater::force_filament_colors_update`
//!   is `#if 0`-ed out (`Plater.cpp:21932`). Same-named presets could never
//!   collapse the palette.
//! - Uniquified names match nothing installed, so `Preset.cpp:1523-1545`
//!   cannot resolve a parent and degrades the preset to defaults.
//!
//! And Orca itself writes **zero** filament preset files in our exact scenario
//! ("stock filament, custom colours"): `_add_project_embedded_presets_to_archive`
//! (`bbs_3mf.cpp:7489`) only iterates presets flagged `is_project_embedded`,
//! which the importer only flags for presets extracted from embedded files.
//! Matching that behaviour is strictly less risky, so this module emits
//! machine + process embedded files and nothing else.

use super::presets::ResolvedSelection;
use serde_json::{Map, Value};
use std::fmt::Write;

/// Build `Metadata/project_settings.config`.
///
/// `palette[i]` is the colour of extruder slot `i + 1` (1-based). `sel` is the
/// machine / process / filament combination the user picked; `None` falls back
/// to a synthetic generic profile so an export still works before the dialog
/// has ever been opened.
pub fn build_project_settings_config(
    palette: &[[u8; 3]],
    sel: Option<&ResolvedSelection<'_>>,
) -> String {
    let colours: Vec<String> = if palette.is_empty() {
        vec![format_hex([138, 138, 138])]
    } else {
        palette.iter().map(|rgb| format_hex(*rgb)).collect()
    };
    let slots = colours.len();

    let mut cfg = Map::new();
    cfg.insert("name".into(), "project_settings".into());
    cfg.insert("from".into(), "project".into());
    cfg.insert("version".into(), PROJECT_VERSION.into());

    match sel {
        Some(sel) => fill_from_selection(&mut cfg, sel, slots),
        None => fill_generic(&mut cfg, slots),
    }

    // Written last so nothing above can overwrite the palette — it is the one
    // array that decides how many extruders the project has.
    cfg.insert("filament_colour".into(), strings(&colours));

    serde_json::to_string_pretty(&Value::Object(cfg)).unwrap_or_else(|_| "{}".to_string())
}

fn fill_from_selection(cfg: &mut Map<String, Value>, sel: &ResolvedSelection<'_>, slots: usize) {
    let printer = &sel.variant.preset_name;
    let process = &sel.process.name;

    // Process settings are scalars and apply to the whole plate.
    for (k, v) in &sel.process.config {
        cfg.insert(k.clone(), v.clone());
    }
    // Printer settings. These come after the process block because the two
    // share a few line-width style keys and the printer's are authoritative.
    for (k, v) in &sel.variant.config {
        cfg.insert(k.clone(), v.clone());
    }

    // Filament settings, transposed: every key any slot mentions becomes an
    // N-length array indexed by slot.
    let mut keys: Vec<&String> = sel
        .filaments
        .iter()
        .flat_map(|f| f.config.keys())
        .collect();
    keys.sort();
    keys.dedup();
    for key in keys {
        let per_slot: Vec<Value> = (0..slots)
            .map(|i| {
                let preset = sel.filaments[i.min(sel.filaments.len() - 1)];
                scalarise(preset.config.get(key))
            })
            .collect();
        cfg.insert(key.clone(), Value::Array(per_slot));
    }

    let filament_names: Vec<String> = (0..slots)
        .map(|i| sel.filaments[i.min(sel.filaments.len() - 1)].name.clone())
        .collect();
    let filament_ids: Vec<String> = (0..slots)
        .map(|i| {
            let p = sel.filaments[i.min(sel.filaments.len() - 1)];
            p.filament_id
                .clone()
                .or_else(|| p.setting_id.clone())
                .unwrap_or_default()
        })
        .collect();

    cfg.insert("filament_settings_id".into(), strings(&filament_names));
    cfg.insert("filament_ids".into(), strings(&filament_ids));
    cfg.insert("print_settings_id".into(), process.clone().into());
    cfg.insert("printer_settings_id".into(), printer.clone().into());
    cfg.insert("default_print_profile".into(), process.clone().into());
    cfg.insert(
        "default_filament_profile".into(),
        strings(&filament_names[..1.min(filament_names.len())]),
    );
    cfg.insert(
        "print_compatible_printers".into(),
        strings(std::slice::from_ref(printer)),
    );
    cfg.insert(
        "inherits_group".into(),
        inherits_group(process, &filament_names, printer),
    );
}

fn fill_generic(cfg: &mut Map<String, Value>, slots: usize) {
    let names = vec![GENERIC_FILAMENT_NAME.to_string(); slots];
    cfg.insert("print_settings_id".into(), PROCESS_PRESET_NAME.into());
    cfg.insert("printer_settings_id".into(), MACHINE_PRESET_NAME.into());
    cfg.insert("default_print_profile".into(), PROCESS_PRESET_NAME.into());
    cfg.insert(
        "default_filament_profile".into(),
        strings(&[GENERIC_FILAMENT_NAME.to_string()]),
    );
    cfg.insert("filament_settings_id".into(), strings(&names));
    cfg.insert(
        "inherits_group".into(),
        inherits_group(PROCESS_PRESET_NAME, &names, MACHINE_PRESET_NAME),
    );

    let repeat = |v: &str| strings(&vec![v.to_string(); slots]);
    cfg.insert("filament_type".into(), repeat("PLA"));
    cfg.insert("filament_vendor".into(), repeat("Generic"));
    cfg.insert("filament_density".into(), repeat("1.24"));
    cfg.insert("filament_diameter".into(), repeat("1.75"));
    cfg.insert("filament_cost".into(), repeat("0"));
    cfg.insert("filament_flow_ratio".into(), repeat("1"));
    cfg.insert("filament_max_volumetric_speed".into(), repeat("15"));
    cfg.insert("filament_soluble".into(), repeat("0"));
    cfg.insert("filament_is_support".into(), repeat("0"));
    // `temperature` and `bed_temperature` are in the obsolete ignore set at
    // PrintConfig.cpp:7268 and would be swallowed without a warning.
    cfg.insert("nozzle_temperature".into(), repeat("220"));
    cfg.insert("nozzle_temperature_initial_layer".into(), repeat("220"));
    cfg.insert("hot_plate_temp".into(), repeat("60"));
    cfg.insert("hot_plate_temp_initial_layer".into(), repeat("60"));
    cfg.insert("cool_plate_temp".into(), repeat("35"));
    cfg.insert("cool_plate_temp_initial_layer".into(), repeat("35"));

    cfg.insert("layer_height".into(), "0.2".into());
    cfg.insert("initial_layer_print_height".into(), "0.2".into());
    cfg.insert("line_width".into(), "0.42".into());
    cfg.insert("printer_technology".into(), "FFF".into());
    cfg.insert("printer_variant".into(), "0.4".into());
    cfg.insert("nozzle_diameter".into(), strings(&["0.4".to_string()]));
    cfg.insert("default_bed_type".into(), "Cool Plate".into()); // coString, PrintConfig.cpp:902
    cfg.insert(
        "printable_area".into(),
        strings(&[
            "0x0".to_string(),
            "220x0".to_string(),
            "220x220".to_string(),
            "0x220".to_string(),
        ]),
    );
    cfg.insert("printable_height".into(), "250".into());
}

/// Build `Metadata/machine_settings_1.config` from a vendor preset (selected
/// path only).
///
/// REFUTE-6: this file is what makes Orca's dropdown show the full preset name
/// ("Snapmaker U1 (0.4 nozzle)") even when the user has not installed the
/// Snapmaker vendor profile. The full flattened vendor config is emitted
/// verbatim so nothing falls back to the default printer's values; we only add
/// the `printer_settings_id` the extractor keys on and the metadata that marks
/// it as project-embedded.
pub fn build_selected_machine_config(sel: &ResolvedSelection<'_>) -> String {
    let mut payload = Map::new();
    payload.insert("name".into(), sel.variant.preset_name.clone().into());
    payload.insert("from".into(), "project".into());
    payload.insert("version".into(), PROJECT_VERSION.into());
    payload.insert(
        "printer_settings_id".into(),
        sel.variant.preset_name.clone().into(),
    );
    for (k, v) in &sel.variant.config {
        payload.insert(k.clone(), v.clone());
    }
    to_pretty(payload)
}

/// Build `Metadata/process_settings_1.config` from a vendor preset (selected
/// path only).
pub fn build_selected_process_config(sel: &ResolvedSelection<'_>) -> String {
    let mut payload = Map::new();
    payload.insert("name".into(), sel.process.name.clone().into());
    payload.insert("from".into(), "project".into());
    payload.insert("version".into(), PROJECT_VERSION.into());
    payload.insert("print_settings_id".into(), sel.process.name.clone().into());
    payload.insert(
        "compatible_printers".into(),
        Value::Array(vec![sel.variant.preset_name.clone().into()]),
    );
    for (k, v) in &sel.process.config {
        payload.insert(k.clone(), v.clone());
    }
    to_pretty(payload)
}

/// Build `Metadata/machine_settings_1.config` (generic fallback path).
///
/// The contents mirror what OrcaSlicer's `_add_project_embedded_presets_to_archive`
/// would write for a stock equivalent of a "Generic Printer 0.4 nozzle"
/// machine profile.
pub fn build_machine_profile_config() -> String {
    let payload = serde_json::json!({
        "name": MACHINE_PRESET_NAME,
        "from": "project",
        "version": PROJECT_VERSION,

        // extractor key (bbs_3mf.cpp:2608). Without this the preset is dropped.
        "printer_settings_id": MACHINE_PRESET_NAME,

        "printer_model": MACHINE_PRESET_NAME,
        "printer_technology": "FFF",
        "printer_variant": "0.4",
        "default_bed_type": "Cool Plate",     // coString (PrintConfig.cpp:902)
        "bed_shape": "0x0,220x0,220x220,0x220",
        "printable_area": "0x0,220x0,220x220,0x220",
        "printable_height": 250,
        "machine_width": 220,
        "machine_depth": 220,
        "machine_height": 250,
        "nozzle_diameter": vec!["0.4".to_string()],
        "nozzle_type": "stainless_steel",
        "nozzle_volume": 117,
        "extruder_type": vec!["DirectDrive"],
        "max_layer_height": vec!["0.28".to_string()],
        "min_layer_height": vec!["0.08".to_string()],
    });
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
}

/// Build `Metadata/process_settings_1.config` (generic fallback path).
///
/// Layer / line width defaults that match the values OrcaSlicer displays when
/// "Default Print Profile" is selected from the Print menu.
pub fn build_process_profile_config() -> String {
    let payload = serde_json::json!({
        "name": PROCESS_PRESET_NAME,
        "from": "project",
        "version": PROJECT_VERSION,

        // extractor key (bbs_3mf.cpp:2587). Without this the preset is dropped.
        "print_settings_id": PROCESS_PRESET_NAME,

        "layer_height": "0.2",
        "line_width": "0.42",
        "initial_layer_print_height": "0.2",
        "initial_layer_line_width": "0.5",
        "inner_wall_line_width": "0.45",
        "outer_wall_line_width": "0.42",
        "print_sequence": "by layer",
        "default_print_profile": PROCESS_PRESET_NAME,
    });
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
}

/// Build `Metadata/filament_settings_<slot>.config` for one slot (generic
/// fallback path).
///
/// `slot` is 1-based (matches paint_color nibble encoding where slot 1 writes
/// `0x4`). `rgb` is the colour assigned to this slot.
pub fn build_filament_profile_config(slot: u8, rgb: [u8; 3]) -> String {
    let preset_name = filament_preset_name(slot);
    let payload = serde_json::json!({
        "name": preset_name,
        "from": "project",
        "version": PROJECT_VERSION,

        // extractor key (bbs_3mf.cpp:2598). Without this the preset is dropped
        // *and* deduplication uses this string as `preset_name`, so two slots
        // with the same value collapse onto one spool.
        "filament_settings_id": vec![preset_name.clone()],

        "filament_type": "PLA",
        "filament_vendor": "Generic",
        "filament_density": vec!["1.24".to_string()],
        "filament_diameter": vec!["1.75".to_string()],
        "filament_colour": vec![format_hex(rgb)],
        "filament_cost": vec!["0".to_string()],
        "filament_ids": vec![format!("F{}", slot)],
        "filament_is_support": vec!["0".to_string()],
        "filament_soluble": vec!["0".to_string()],
        "filament_max_volumetric_speed": vec!["15".to_string()],
        "filament_flow_ratio": vec!["1".to_string()],

        // Per filament; feed `nozzle_temperature` instead of the obsolete
        // `temperature` field that lives in PrintConfig.cpp:7268's ignore set.
        "nozzle_temperature": vec!["210".to_string()],
        "nozzle_temperature_initial_layer": vec!["210".to_string()],
        "nozzle_temperature_range_high": vec!["240".to_string()],
        "nozzle_temperature_range_low": vec!["190".to_string()],

        // Spool card "bed temperature" cell reads these (coString / coFloats).
        // `bed_temperature` is also obsolete (PrintConfig.cpp:7268).
        "hot_plate_temp": vec!["60".to_string()],
        "hot_plate_temp_initial_layer": vec!["60".to_string()],
        "cool_plate_temp": vec!["35".to_string()],
        "cool_plate_temp_initial_layer": vec!["35".to_string()],
        "bed_temperature_difference": vec!["10".to_string()],
    });
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
}

/// `Metadata/machine_settings_<n>.config` (1-based).
pub const MACHINE_PRESET_PATH: &str = "Metadata/machine_settings_1.config";

/// `Metadata/process_settings_<n>.config` (1-based).
pub const PROCESS_PRESET_PATH: &str = "Metadata/process_settings_1.config";

/// `Metadata/filament_settings_<slot>.config` for 1-based `slot`.
pub fn filament_preset_path(slot: u8) -> String {
    let mut s = String::from("Metadata/filament_settings_");
    let _ = write!(s, "{}.config", slot);
    s
}

/// Preset names are stable strings; both the project settings ID and the
/// embedded preset's `filament_settings_id[]` use the same identifier so the
/// loader can match them.
pub const MACHINE_PRESET_NAME: &str = "Generic Printer";
pub const PROCESS_PRESET_NAME: &str = "0.20mm Standard";
pub const GENERIC_FILAMENT_NAME: &str = "Generic PLA";
pub const PROJECT_VERSION: &str = "0.1.0";

/// One preset per slot. The `@slot<N>` suffix is required (see REFUTE-4 in
/// the module-level docs) — duplicates collapse.
pub fn filament_preset_name(slot: u8) -> String {
    let mut s = String::from("Generic PLA @slot");
    let _ = write!(s, "{}", slot);
    s
}

/// `[process, filament_1 .. filament_N, printer]` — the exact `num_filaments+2`
/// layout the loader indexes at `PresetBundle.cpp:2861/2876/2943`.
fn inherits_group(process: &str, filaments: &[String], printer: &str) -> Value {
    let mut group = Vec::with_capacity(filaments.len() + 2);
    group.push(Value::String(process.to_string()));
    group.extend(filaments.iter().map(|f| Value::String(f.clone())));
    group.push(Value::String(printer.to_string()));
    Value::Array(group)
}

/// Vendor presets store per-extruder values as one-element arrays. When we
/// transpose them into a project-level array we need the value itself, not a
/// nested array.
fn scalarise(v: Option<&Value>) -> Value {
    match v {
        Some(Value::Array(a)) => a.first().cloned().unwrap_or(Value::String(String::new())),
        Some(other) => other.clone(),
        None => Value::String(String::new()),
    }
}

fn strings(v: &[String]) -> Value {
    Value::Array(v.iter().map(|s| Value::String(s.clone())).collect())
}

fn to_pretty(map: Map<String, Value>) -> String {
    serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_else(|_| "{}".to_string())
}

/// `#RRGGBB`, upper case. `Preset.cpp:1024`'s colour table parses lower case
/// hex as zero in release builds, so a lower case value silently becomes black.
fn format_hex(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::presets::ExportSelection;

    fn parse(json: &str) -> Value {
        serde_json::from_str(json).expect("config must be valid JSON")
    }

    fn generic(palette: &[[u8; 3]]) -> Value {
        parse(&build_project_settings_config(palette, None))
    }

    fn selection(filaments: Vec<&str>) -> ExportSelection {
        ExportSelection {
            machine_id: "snapmaker_u1".into(),
            nozzle_diameter: "0.4".into(),
            process_name: "0.20 Standard @Snapmaker U1 (0.4 nozzle)".into(),
            filament_names: filaments.into_iter().map(String::from).collect(),
            target_slicer: "orcaslicer".into(),
        }
    }

    fn selected(palette: &[[u8; 3]], filaments: Vec<&str>) -> Value {
        let sel = selection(filaments);
        let resolved = sel.resolve(palette.len()).expect("selection must resolve");
        parse(&build_project_settings_config(palette, Some(&resolved)))
    }

    #[test]
    fn filament_colour_length_matches_the_palette() {
        let v = generic(&[[255, 0, 0], [0, 255, 0], [0, 0, 255]]);
        let colours = v["filament_colour"].as_array().unwrap();
        assert_eq!(colours.len(), 3);
        assert_eq!(colours[0], "#FF0000");
        assert_eq!(colours[2], "#0000FF");
    }

    #[test]
    fn an_empty_palette_still_declares_one_filament() {
        assert_eq!(generic(&[])["filament_colour"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn hex_is_upper_case() {
        assert_eq!(format_hex([0xAB, 0xCD, 0xEF]), "#ABCDEF");
    }

    #[test]
    fn inherits_group_is_num_filaments_plus_two() {
        // PresetBundle.cpp:2861 reads [0] as the process parent and :2876 reads
        // [N+1] as the printer's; a short array would silently shift both.
        for slots in 1..=6 {
            let palette = vec![[1, 2, 3]; slots];
            let v = generic(&palette);
            let g = v["inherits_group"].as_array().unwrap();
            assert_eq!(g.len(), slots + 2, "{} slots", slots);
            assert_eq!(g[0], PROCESS_PRESET_NAME);
            assert_eq!(g[slots + 1], MACHINE_PRESET_NAME);
        }
    }

    #[test]
    fn selected_export_names_the_real_vendor_presets() {
        let v = selected(&[[255, 0, 0], [0, 0, 255]], vec!["Generic PLA"]);
        assert_eq!(v["printer_settings_id"], "Snapmaker U1 (0.4 nozzle)");
        assert_eq!(
            v["print_settings_id"],
            "0.20 Standard @Snapmaker U1 (0.4 nozzle)"
        );
        // printer_model lives in the embedded machine config, not the project
        // file — it is not a PrintConfigDef key on this path.
        assert!(v.get("printer_model_id").is_none());
    }

    #[test]
    fn the_embedded_machine_config_is_a_complete_preset() {
        let sel = selection(vec!["Generic PLA"]);
        let resolved = sel.resolve(1).expect("selection must resolve");
        let v = parse(&build_selected_machine_config(&resolved));
        // REFUTE-6: the embedded machine preset is what makes the dropdown show
        // the full name; the extractor keys on printer_settings_id.
        assert_eq!(v["printer_settings_id"], "Snapmaker U1 (0.4 nozzle)");
        assert_eq!(v["printer_model"], "Snapmaker U1");
        assert_eq!(v["printer_variant"], "0.4");
        // REFUTE-6: it must be complete — gcode macros included, otherwise the
        // fallback to the default printer silently replaces them.
        assert!(v.get("machine_start_gcode").is_some());
        assert!(v.get("machine_end_gcode").is_some());
        assert_eq!(v["nozzle_diameter"].as_array().unwrap().len(), 4);
        // 4-extruder arrays must not shrink to the palette length.
        assert_eq!(v["extruder_offset"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn the_embedded_process_config_links_to_the_machine() {
        let sel = selection(vec!["Generic PLA"]);
        let resolved = sel.resolve(1).unwrap();
        let v = parse(&build_selected_process_config(&resolved));
        assert_eq!(v["print_settings_id"], "0.20 Standard @Snapmaker U1 (0.4 nozzle)");
        // compatible_printers pins the process to the machine variant so the
        // process stays selectable after load.
        let cp = v["compatible_printers"].as_array().unwrap();
        assert_eq!(cp[0], "Snapmaker U1 (0.4 nozzle)");
        assert_eq!(v["layer_height"], "0.2");
    }

    #[test]
    fn per_slot_filament_values_are_full_length_arrays() {
        // PresetBundle.cpp:2925 reads index i of every vector option for slot
        // i, so a mixed-material project has to transpose its presets here.
        let v = selected(
            &[[255, 0, 0], [0, 0, 255]],
            vec!["Generic PLA", "Generic PETG"],
        );
        let types = v["filament_type"].as_array().unwrap();
        assert_eq!(types.len(), 2);
        assert_eq!(types[0], "PLA");
        assert_eq!(types[1], "PETG");
        // The transposed values must be scalars, not the vendor preset's
        // one-element arrays.
        assert!(types[0].is_string());
    }

    #[test]
    fn one_filament_choice_spreads_over_every_slot() {
        let palette = vec![[1, 2, 3], [4, 5, 6], [7, 8, 9], [10, 11, 12]];
        let v = selected(&palette, vec!["Generic PLA"]);
        for key in ["filament_type", "filament_settings_id", "nozzle_temperature"] {
            assert_eq!(
                v[key].as_array().unwrap().len(),
                4,
                "{} must cover every slot",
                key
            );
        }
        assert_eq!(v["inherits_group"].as_array().unwrap().len(), 6);
    }

    #[test]
    fn the_machine_geometry_survives_into_the_project_config() {
        let v = selected(&[[1, 2, 3]], vec!["Generic PLA"]);
        assert_eq!(v["printable_height"], "270.05");
        assert_eq!(v["default_bed_type"], "Textured PEI Plate");
        // Four extruders: the machine's per-extruder arrays must stay 4 long
        // regardless of how many colour slots the palette has.
        assert_eq!(v["nozzle_diameter"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn obsolete_temperature_keys_are_never_emitted() {
        // Both are in the obsolete ignore set (PrintConfig.cpp:7268) and would
        // be dropped without any warning.
        for v in [
            generic(&[[1, 2, 3]]),
            selected(&[[1, 2, 3]], vec!["Generic PLA"]),
        ] {
            assert!(v.get("temperature").is_none());
            assert!(v.get("bed_temperature").is_none());
            assert!(v.get("nozzle_temperature").is_some());
            assert!(v.get("hot_plate_temp").is_some());
        }
    }

    #[test]
    fn no_slot_suffixed_preset_names_are_invented() {
        // REFUTE-7: a name the user does not have installed cannot resolve a
        // parent, and Preset.cpp:1543 then replaces every value with the
        // default filament's.
        let v = selected(&[[1, 2, 3], [4, 5, 6]], vec!["Generic PLA"]);
        let ids = v["filament_settings_id"].as_array().unwrap();
        assert!(ids.iter().all(|n| n == "Generic PLA"));
        assert!(!serde_json::to_string(&v).unwrap().contains("@slot"));
    }
}
