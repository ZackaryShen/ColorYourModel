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
    let settings_ids: Vec<String> = (1..=count).map(|i| format!("Generic PLA {}", i)).collect();
    let types: Vec<String> = vec!["PLA".to_string(); count];

    let payload = serde_json::json!({
        "filament_colour": colours,
        "filament_settings_id": settings_ids,
        "filament_type": types,
        "from": "ColorYourModel",
    });

    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string())
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
        for key in ["filament_colour", "filament_settings_id", "filament_type"] {
            assert_eq!(
                v[key].as_array().unwrap().len(),
                2,
                "{} has the wrong length",
                key
            );
        }
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
