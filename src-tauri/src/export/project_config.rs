//! Writer for `Metadata/project_settings.config`, the JSON blob that tells the
//! slicer what the extruder slots referenced by `paint_color` actually look
//! like.
//!
//! Without this entry the Snapmaker fork's
//! `max_supported_filament_id_from_project_config()` finds no filament keys,
//! returns `INT_MAX`, and every slot falls back to whatever palette the user
//! happens to have loaded, so the exported colours are silently replaced.
//! Upstream stock reads the same path (`BBS_PROJECT_CONFIG_FILE`), so writing
//! it is required on both trees, not just the fork.
//!
//! The lookup order in `physical_filament_count_from_project_config()` is
//! `filament_colour`, `filament_settings_id`, `filament_ids`,
//! `default_filament_colour`, then `nozzle_diameter`. We populate the first
//! two; every value is a string or an array of strings because that is what
//! `ConfigOptionStrings` expects and what `save_to_json` emits on the slicer
//! side.

/// Build the JSON payload for `Metadata/project_settings.config`.
///
/// `palette[i]` describes extruder slot `i + 1`, matching the slot numbering
/// produced by [`crate::export::quantize`].
///
/// What the slicer reads from this file is not just the count of slots: every
/// spool card on the OrcaSlicer "Print" panel (name, vendor, diameter,
/// density, temperature, cost, ...) reads from these fields. When the project
/// settings are sparse, OrcaSlicer silently falls back to the global user
/// preset, which is why opening the export of a freshly coloured model shows
/// "Generic PLA" with blank details and only the colour we wrote. The reference
/// is the project_settings.config in `pa_pattern.3mf` shipped with Snapmaker
/// Orca 2.3.3 (head `/c/workDir/programing/OrcaHunSe/OrcaSlicer/build/.../`):
/// every project dumps its full print, filament, and machine profile into the
/// one file. We write the fields the spool card actually needs.
pub fn build_project_settings_config(palette: &[[u8; 3]]) -> String {
    let colours: Vec<String> = palette.iter().map(|rgb| format_hex(*rgb)).collect();

    // A slicer that finds zero filaments treats the file as unbounded, so keep
    // at least one entry even for a mesh that somehow carries no colour.
    let colours = if colours.is_empty() {
        vec![format_hex([138, 138, 138])]
    } else {
        colours
    };

    let count = colours.len();
    // Per-slot arrays. Kept as Vec<String> so the slicer parses them through
    // `ConfigOptionStrings`, which is what `save_to_json` emits on the slicer
    // side and what `load_from_json` expects back.
    let settings_ids: Vec<String> = (1..=count).map(|i| format!("Generic PLA {}", i)).collect();
    let types: Vec<String> = vec!["PLA".to_string(); count];
    let vendors: Vec<String> = vec!["Generic".to_string(); count];
    let densities: Vec<String> = vec!["1.24".to_string(); count];
    let diameters: Vec<String> = vec!["1.75".to_string(); count];
    let costs: Vec<String> = vec!["0".to_string(); count];
    let ids: Vec<String> = (1..=count).map(|i| format!("F{}", i)).collect();
    let is_support: Vec<String> = vec!["0".to_string(); count];
    let soluble: Vec<String> = vec!["0".to_string(); count];
    let max_volumetric_speed: Vec<String> = vec!["15".to_string(); count];
    let flow_ratios: Vec<String> = vec!["1".to_string(); count];
    let nozzle_diameters: Vec<String> = vec!["0.4".to_string(); count];
    let nozzle_temps: Vec<String> = vec!["210".to_string(); count];
    let nozzle_temps_first_layer: Vec<String> = vec!["210".to_string(); count];
    let nozzle_temp_highs: Vec<String> = vec!["240".to_string(); count];
    let nozzle_temp_lows: Vec<String> = vec!["190".to_string(); count];
    let bed_temps_first_layer: Vec<String> = vec!["60".to_string(); count];
    let cool_plate_temps_first_layer: Vec<String> = vec!["35".to_string(); count];
    let bed_temp_diffs: Vec<String> = vec!["10".to_string(); count];
    let chamber_temps: Vec<String> = vec!["0".to_string(); count];
    let extruder_types: Vec<String> = vec!["DirectDrive".to_string(); count];
    let extruder_offsets: Vec<String> = vec!["0x0".to_string(); count];
    let extruder_colours: Vec<String> = colours.clone();

    let payload = serde_json::json!({
        "name": "project_settings",
        "from": "ColorYourModel",
        "version": "0.1.0",

        // Defaults applied when an embedded preset cannot be located by id.
        "default_filament_profile": ["Generic PLA"],
        "default_filament_colour": default_filament_colour(&colours),
        "default_print_profile": "0.20mm Standard",

        // Per-slot spool metadata. Lengths must equal `count`.
        "filament_colour": colours,
        "filament_settings_id": settings_ids,
        "filament_type": types,
        "filament_vendor": vendors,
        "filament_density": densities,
        "filament_diameter": diameters,
        "filament_cost": costs,
        "filament_ids": ids,
        "filament_is_support": is_support,
        "filament_soluble": soluble,
        "filament_max_volumetric_speed": max_volumetric_speed,
        "filament_flow_ratio": flow_ratios,

        // Print process profile id and the layer/line defaults the slicer
        // shows in Quality when no embedded process profile is loaded.
        "print_settings_id": "0.20mm Standard",
        "printer_settings_id": "Generic Printer",
        "print_compatible_printers": ["Generic Printer 0.4 nozzle"],
        "layer_height": "0.2",
        "line_width": "0.42",
        "initial_layer_print_height": "0.2",
        "initial_layer_line_width": "0.5",
        "inner_wall_line_width": "0.45",
        "outer_wall_line_width": "0.42",

        // Machine and nozzle. The per-slot `nozzle_diameter`/`nozzle_temperature`
        // arrays are the ones OrcaSlicer actually reads; the scalar fields
        // below keep the global "Printer" tab populated.
        "nozzle_diameter": nozzle_diameters,
        "nozzle_type": "stainless_steel",
        "nozzle_volume": "117",
        "nozzle_temperature": nozzle_temps,
        "nozzle_temperature_initial_layer": nozzle_temps_first_layer,
        "nozzle_temperature_range_high": nozzle_temp_highs,
        "nozzle_temperature_range_low": nozzle_temp_lows,
        "extruder_type": extruder_types,
        "extruder_offset": extruder_offsets,
        "extruder_colour": extruder_colours,

        // Bed / chamber. Per-slot arrays match the per-slot nozzle arrays.
        "hot_plate_temp_initial_layer": bed_temps_first_layer,
        "cool_plate_temp_initial_layer": cool_plate_temps_first_layer,
        "bed_temperature_difference": bed_temp_diffs,
        "chamber_temperatures": chamber_temps,
    });

    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
}

/// First spool colour if non-empty, else the project's neutral default. Orca's
/// `default_filament_colour` is read as an array but is normally a singleton.
fn default_filament_colour(colours: &[String]) -> Vec<String> {
    vec![colours.first().cloned().unwrap_or_else(|| "#8A8A8A".to_string())]
}

/// `#RRGGBB`, upper case. Lower case hex is parsed as zero by the release
/// build of the slicer's colour table.
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
    fn parallel_arrays_stay_the_same_length() {
        let palette = vec![[1, 2, 3], [4, 5, 6]];
        let v = parse(&build_project_settings_config(&palette));
        for key in [
            "filament_colour",
            "filament_settings_id",
            "filament_type",
            "filament_vendor",
            "filament_density",
            "filament_diameter",
            "filament_ids",
            "nozzle_diameter",
            "extruder_colour",
        ] {
            assert_eq!(
                v[key].as_array().unwrap().len(),
                2,
                "{} has the wrong length",
                key
            );
        }
    }

    #[test]
    fn meta_keys_are_scalar_strings() {
        let palette = vec![[10, 20, 30], [40, 50, 60]];
        let v = parse(&build_project_settings_config(&palette));
        for key in [
            "name",
            "from",
            "version",
            "print_settings_id",
            "printer_settings_id",
            "default_print_profile",
        ] {
            assert!(v[key].is_string(), "{} should be a scalar string", key);
        }
        assert_eq!(v["name"], "project_settings");
        assert_eq!(v["from"], "ColorYourModel");
    }

    #[test]
    fn spool_metadata_fields_are_present() {
        let v = parse(&build_project_settings_config(&[[1, 2, 3], [4, 5, 6]]));
        for key in [
            "filament_vendor",
            "filament_density",
            "filament_diameter",
            "filament_cost",
            "filament_max_volumetric_speed",
            "filament_flow_ratio",
            "filament_is_support",
            "filament_soluble",
            "nozzle_temperature",
            "nozzle_type",
            "extruder_type",
            "print_compatible_printers",
        ] {
            assert!(v.get(key).is_some(), "{} missing from config", key);
        }
    }

    #[test]
    fn print_compatible_printers_listing_is_present_and_non_empty() {
        let v = parse(&build_project_settings_config(&[[10, 20, 30]]));
        let arr = v["print_compatible_printers"].as_array().expect("array");
        assert!(!arr.is_empty());
        assert!(arr[0]
            .as_str()
            .unwrap()
            .contains("Generic Printer"));
    }

    #[test]
    fn hex_is_upper_case() {
        let palette = vec![[0xAB, 0xCD, 0xEF]];
        let v = parse(&build_project_settings_config(&palette));
        assert_eq!(v["filament_colour"][0], "#ABCDEF");
    }

    #[test]
    fn an_empty_palette_still_declares_one_filament() {
        // Zero filaments makes max_supported_filament_id return INT_MAX, which
        // disables the very clamping this file exists to drive.
        let v = parse(&build_project_settings_config(&[]));
        assert_eq!(v["filament_colour"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn every_value_is_a_string_so_ConfigOptionStrings_can_parse_it() {
        let palette = vec![[10, 20, 30], [40, 50, 60]];
        let v = parse(&build_project_settings_config(&palette));
        for key in ["filament_colour", "filament_settings_id", "filament_type"] {
            for item in v[key].as_array().unwrap() {
                assert!(item.is_string(), "{} contains a non-string entry", key);
            }
        }
    }
}
