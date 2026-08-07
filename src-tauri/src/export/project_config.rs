//! Writers for the JSON blobs OrcaSlicer reads when loading an exported `.3mf`.
//!
//! Files emitted:
//! - `Metadata/project_settings.config` — the project-level "what printer,
//!   what process, what colour map" snapshot. References the IDs of the
//!   embedded preset files below.
//! - `Metadata/machine_settings_1.config` — embedded printer profile. Carries
//!   `nozzle_diameter`, `printable_area`, `default_bed_type`, etc.
//! - `Metadata/process_settings_1.config` — embedded process profile.
//!   `layer_height`, `line_width`, `print_settings_id`.
//! - `Metadata/filament_settings_<i>.config` for `i in 1..=N` — one embedded
//!   filament preset per extruder slot.
//!
//! ## Why the embedded files are required
//!
//! Snapmaker Orca's `bbs_3mf.cpp:_extract_project_embedded_presets_from_archive`
//! (upstream stock equivalent at the same offset) reads each embedded `.config`
//! and registers it as a project preset. The spool card on the Orca UI reads
//! `filament_type`, `filament_vendor`, `filament_density`, `filament_diameter`,
//! `nozzle_temperature`, `hot_plate_temp` from this preset. Without the file,
//! OrcaSlicer falls back to the system generic preset, which only contains a
//! name and a colour, so the spool card opens with empty type/vendor/density
//! boxes even though every field is sitting in `project_settings.config`.
//!
//! ## Name duplication rule (REFUTE-4)
//!
//! `_extract_project_embedded_presets_from_archive` records `preset_name =
//! filament_settings_id.values[0]` (see `bbs_3mf.cpp:2598`). If six embedded
//! filament presets all declare the same name, OrcaSlicer deduplicates them
//! and only one spool is registered, so the six colours collapse onto a
//! single filament. Each slot therefore gets a unique preset name
//! (`"Generic PLA @slot<1..N>"`).
//!
//! ## Field selection (REFUTE-a/c)
//!
//! OrcaSlicer's `load_from_json` (`Config.cpp:792`) routes metadata keys
//! (`version`, `name`, `from`, `is_custom`, `type`, `setting_id`,
//! `filament_id`, `url`, `description`, `instantiation`, `inherits`) into
//! `key_values` instead of the config. Everything else is deserialised
//! through `set_deserialize` against `PrintConfigDef`. Two consequences:
//!
//! - The embedded printer preset must carry `printer_settings_id`; the
//!   process preset must carry `print_settings_id`; the filament preset
//!   must carry `filament_settings_id` (an array). Without these, the
//!   extractor at `bbs_3mf.cpp:2596-2611` returns early and silently
//!   drops the file.
//! - `temperature` and `bed_temperature` are NOT in `PrintConfigDef`; both
//!   are listed in the obsolete ignore set at `PrintConfig.cpp:7268` and
//!   would be silently swallowed. Use `nozzle_temperature` and
//!   `hot_plate_temp` instead.

use std::fmt::Write;

/// Build `Metadata/project_settings.config`.
///
/// `palette[i]` is the colour assigned to extruder slot `i + 1` (1-based).
/// The IDs the file declares (`print_settings_id`, `printer_settings_id`,
/// `filament_settings_id[]`) match the names of the embedded preset files
/// emitted alongside.
pub fn build_project_settings_config(palette: &[[u8; 3]]) -> String {
    let count = palette.len().max(1);
    let colours: Vec<String> = palette
        .iter()
        .map(|rgb| format_hex(*rgb))
        .collect();
    let colours = if colours.is_empty() {
        vec![format_hex([138, 138, 138])]
    } else {
        colours
    };
    let settings_ids: Vec<String> = (1..=count)
        .map(|i| filament_preset_name(i.try_into().expect("palette exceeds 255 slots")))
        .collect();

    let payload = serde_json::json!({
        "name": "project_settings",
        "from": "project",
        "version": PROJECT_VERSION,

        "default_filament_profile": settings_ids.first().cloned().map(|s| vec![s]),
        "default_filament_colour": vec![colours.first().cloned().unwrap_or_else(|| "#8A8A8A".to_string())],
        "default_print_profile": PROCESS_PRESET_NAME,

        "filament_colour": colours,
        "filament_settings_id": settings_ids.clone(),

        "print_settings_id": PROCESS_PRESET_NAME,
        "printer_settings_id": MACHINE_PRESET_NAME,
        "print_compatible_printers": vec![format!("{} 0.4 nozzle", MACHINE_PRESET_NAME)],
    });

    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
}

/// Build `Metadata/machine_settings_1.config`.
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

/// Build `Metadata/process_settings_1.config`.
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

/// Build `Metadata/filament_settings_<slot>.config` for one slot.
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
pub const PROJECT_VERSION: &str = "0.1.0";

/// One preset per slot. The `@slot<N>` suffix is required (see REFUTE-4 in
/// the module-level docs) — duplicates collapse.
pub fn filament_preset_name(slot: u8) -> String {
    let mut s = String::from("Generic PLA @slot");
    let _ = write!(s, "{}", slot);
    s
}

/// `#RRGGBB`, upper case. Lower case hex is parsed as zero by OrcaSlicer's
/// `SetFilamentColor()` colour table in release builds (`#000000` falls back
/// to filament black). See `Preset.cpp:1024` `opt_color_to_int_map`.
fn format_hex(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> serde_json::Value {
        serde_json::from_str(json).expect("config must be valid JSON")
    }

    #[test]
    fn filament_colour_length_matches_the_palette() {
        let palette = vec![[255, 0, 0], [0, 255, 0], [0, 0, 255]];
        let v = parse(&build_project_settings_config(&palette));
        let colours = v["filament_colour"].as_array().unwrap();
        assert_eq!(colours.len(), 3);
        assert_eq!(colours[0], "#FF0000");
        assert_eq!(colours[1], "#00FF00");
        assert_eq!(colours[2], "#0000FF");
    }

    #[test]
    fn filament_settings_id_matches_per_slot() {
        let palette = vec![[1, 2, 3], [4, 5, 6]];
        let v = parse(&build_project_settings_config(&palette));
        let ids = v["filament_settings_id"].as_array().unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], "Generic PLA @slot1");
        assert_eq!(ids[1], "Generic PLA @slot2");
    }

    #[test]
    fn machine_preset_contains_printer_settings_id() {
        let v = parse(&build_machine_profile_config());
        // Required by the extractor (bbs_3mf.cpp:2608).
        assert_eq!(v["printer_settings_id"], MACHINE_PRESET_NAME);
        // Default bed type is a coString per PrintConfig.cpp:902, not a number.
        assert_eq!(v["default_bed_type"], "Cool Plate");
        // nozzle_diameter must be an array, never a scalar.
        let nd = v["nozzle_diameter"].as_array().expect("array");
        assert_eq!(nd.len(), 1);
    }

    #[test]
    fn process_preset_contains_print_settings_id() {
        let v = parse(&build_process_profile_config());
        // Required by the extractor (bbs_3mf.cpp:2587).
        assert_eq!(v["print_settings_id"], PROCESS_PRESET_NAME);
    }

    #[test]
    fn filament_preset_carries_per_slot_metadata() {
        let v = parse(&build_filament_profile_config(2, [10, 20, 30]));
        let name = filament_preset_name(2);
        assert_eq!(v["name"], name);
        // extractor key (bbs_3mf.cpp:2598). ARRAY — not scalar.
        let ids = v["filament_settings_id"].as_array().expect("array");
        assert_eq!(ids[0], name);
        // bed temperature uses hot_plate_temp, NOT the obsolete bed_temperature
        assert!(
            v.get("bed_temperature").is_none(),
            "bed_temperature is in the obsolete ignore set and must be omitted"
        );
        assert!(v.get("hot_plate_temp").is_some());
        // temperature is also obsolete; nozzle_temperature is the real field.
        assert!(v.get("temperature").is_none());
        assert!(v["nozzle_temperature"].is_array());
    }

    #[test]
    fn filament_preset_names_are_unique_per_slot() {
        let names: Vec<String> = (1..=4)
            .map(|i| filament_preset_name(i))
            .collect();
        let unique: std::collections::HashSet<&String> = names.iter().collect();
        assert_eq!(unique.len(), 4, "duplicate preset names collapse in ORCA");
    }

    #[test]
    fn hex_is_upper_case() {
        assert_eq!(format_hex([0xAB, 0xCD, 0xEF]), "#ABCDEF");
    }

    #[test]
    fn an_empty_palette_still_declares_one_filament() {
        let v = parse(&build_project_settings_config(&[]));
        assert_eq!(v["filament_colour"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn every_value_is_a_string_so_configoptionstrings_can_parse_it() {
        let v = parse(&build_project_settings_config(&[[10, 20, 30], [40, 50, 60]]));
        for key in ["filament_colour", "filament_settings_id"] {
            for item in v[key].as_array().unwrap() {
                assert!(item.is_string(), "{} contains a non-string entry", key);
            }
        }
    }
}
