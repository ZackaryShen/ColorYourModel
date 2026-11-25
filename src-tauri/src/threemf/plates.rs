//! Plate grouping from `Metadata/model_settings.config`.
//!
//! Orca/Bambu-family slicers record which object prints on which plate in a
//! side-car XML part. The reader only needs the object metadata (name,
//! extruder) and the plate membership that drives [`crate::threemf::plates`]'s
//! grouping; the per-plate thumbnails, filament maps and the `<assemble>`
//! block are ignored.
//!
//! The XML is untrusted: entries with unparsable ids are skipped, plate
//! metadata falls back to a 1-based ordinal, and a document without a complete
//! `<config>` element is an error. Nothing here panics on bad input.

use std::collections::HashMap;

use quick_xml::events::{BytesEnd, BytesStart, Event};
use quick_xml::Reader;

use crate::threemf::model::BuildItem;

/// Object metadata recorded in the config, keyed by object id.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ObjectMeta {
    pub name: Option<String>,
    pub extruder: Option<u8>,
}

/// The parts of `model_settings.config` the reader consumes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelSettings {
    pub objects: HashMap<u32, ObjectMeta>,
    pub plates: Vec<Plate>,
    /// Whether the config described at least one plate. A config without any
    /// `<plate>` element carries no plate information, so grouping falls back
    /// to a single plate.
    pub present: bool,
}

/// One plate as it appears in the config.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Plate {
    /// `plater_id` when it parses, else the plate's 1-based file ordinal.
    pub plater_id: u32,
    /// `plater_name`, empty when absent (the caller falls back to object/file
    /// names).
    pub name: String,
    /// Object ids listed by this plate's `model_instance` blocks, in file order.
    pub object_ids: Vec<u32>,
}

/// The build items assigned to one plate, as indices into the original build.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlateGroup {
    /// `None` when the file had no plate information at all.
    pub plater_id: Option<u32>,
    pub name: String,
    pub item_indices: Vec<usize>,
}

/// Parse the `Metadata/model_settings.config` part.
///
/// Returns `Err` only when the bytes are not a complete `<config>` document;
/// unparsable ids and missing metadata produce skipped entries or defaults.
///
/// `present` is set when the config described at least one plate: a config
/// without plates carries no membership information, so [`group`] treats it
/// like a missing config.
pub fn parse(xml: &[u8]) -> Result<ModelSettings, String> {
    let mut parser = ConfigParser::default();
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => parser.start(&e),
            Ok(Event::Empty(e)) => parser.empty(&e),
            Ok(Event::End(e)) => parser.end(&e),
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(format!("model_settings.config: malformed XML: {e}")),
        }
        buf.clear();
    }
    if !(parser.config_opened && parser.config_closed) {
        return Err("model_settings.config: XML has no complete <config> element".to_string());
    }
    Ok(ModelSettings {
        objects: parser.objects,
        present: !parser.plates.is_empty(),
        plates: parser.plates,
    })
}

/// Group build items by the plate that lists their object.
///
/// Grouping is purely membership based: an item joins the plate whose
/// `model_instance` list contains its object id, in build order. Items listed
/// on no plate join the first group (never dropped) and are warned about. A
/// plate with no matching items still yields an empty group.
pub fn group(build: &[BuildItem], settings: &ModelSettings) -> Vec<PlateGroup> {
    if !settings.present || settings.plates.is_empty() {
        return vec![PlateGroup {
            plater_id: None,
            name: "plate 1".to_string(),
            item_indices: (0..build.len()).collect(),
        }];
    }

    let mut groups: Vec<PlateGroup> = settings
        .plates
        .iter()
        .map(|plate| PlateGroup {
            plater_id: Some(plate.plater_id),
            name: plate.name.clone(),
            item_indices: Vec::new(),
        })
        .collect();

    // First plate wins when two plates list the same object: an object prints
    // on one plate, and a duplicated listing is a file anomaly, not a request
    // to print it twice.
    let mut plate_of: HashMap<u32, usize> = HashMap::new();
    for (index, plate) in settings.plates.iter().enumerate() {
        for &object_id in &plate.object_ids {
            plate_of.entry(object_id).or_insert(index);
        }
    }

    for (index, item) in build.iter().enumerate() {
        match plate_of.get(&item.object_id) {
            Some(&plate) => groups[plate].item_indices.push(index),
            None => {
                log::warn!(
                    "build item {index} (object {}) is listed on no plate; assigning it to the first plate",
                    item.object_id
                );
                groups[0].item_indices.push(index);
            }
        }
    }
    groups
}

/// One XML element that is still open, from the config element inwards.
enum Scope {
    Object {
        id: Option<u32>,
        meta: ObjectMeta,
    },
    Part,
    Plate {
        plater_id: Option<u32>,
        name: String,
        object_ids: Vec<u32>,
    },
    ModelInstance {
        object_id: Option<u32>,
    },
    Other,
}

/// Parser state for one config document.
#[derive(Default)]
struct ConfigParser {
    objects: HashMap<u32, ObjectMeta>,
    plates: Vec<Plate>,
    scopes: Vec<Scope>,
    config_opened: bool,
    config_closed: bool,
}

impl ConfigParser {
    fn start(&mut self, e: &BytesStart) {
        match e.local_name().as_ref() {
            b"config" => {
                self.config_opened = true;
                self.scopes.push(Scope::Other);
            }
            b"object" => {
                let id = attribute(e, b"id").and_then(|value| value.trim().parse().ok());
                self.scopes.push(Scope::Object {
                    id,
                    meta: ObjectMeta::default(),
                });
            }
            b"part" => self.scopes.push(Scope::Part),
            b"plate" => self.scopes.push(Scope::Plate {
                plater_id: None,
                name: String::new(),
                object_ids: Vec::new(),
            }),
            b"model_instance" => self.scopes.push(Scope::ModelInstance { object_id: None }),
            b"metadata" => self.metadata(e),
            _ => self.scopes.push(Scope::Other),
        }
    }

    fn empty(&mut self, e: &BytesStart) {
        match e.local_name().as_ref() {
            // Leaves: handled without opening a scope, so they cannot unbalance
            // the stack.
            b"metadata" => self.metadata(e),
            b"config" => {
                self.config_opened = true;
                self.config_closed = true;
            }
            // Structural elements that are empty in a real file: run the same
            // start/finish path so they yield their (empty) entry.
            b"object" | b"part" | b"plate" | b"model_instance" => {
                self.start(e);
                self.finish();
            }
            _ => {}
        }
    }

    fn end(&mut self, e: &BytesEnd) {
        match e.local_name().as_ref() {
            b"config" => {
                self.config_closed = true;
                self.finish();
            }
            // Leaves never opened a scope, so their end tag must not pop one.
            b"metadata" => {}
            _ => self.finish(),
        }
    }

    /// Close the innermost element and record whatever it contributed.
    fn finish(&mut self) {
        match self.scopes.pop() {
            Some(Scope::Object { id: Some(id), meta }) => {
                // First appearance wins: a duplicated object block cannot
                // silently replace the metadata the plate listing refers to.
                self.objects.entry(id).or_insert(meta);
            }
            Some(Scope::Plate {
                plater_id,
                name,
                object_ids,
            }) => {
                let ordinal = self.plates.len() as u32 + 1;
                self.plates.push(Plate {
                    plater_id: plater_id.unwrap_or(ordinal),
                    name,
                    object_ids,
                });
            }
            Some(Scope::ModelInstance {
                object_id: Some(id),
            }) => {
                if let Some(Scope::Plate { object_ids, .. }) = self
                    .scopes
                    .iter_mut()
                    .rev()
                    .find(|scope| matches!(scope, Scope::Plate { .. }))
                {
                    object_ids.push(id);
                }
            }
            _ => {}
        }
    }

    /// Record one `<metadata key=... value=...>`, attributed to the nearest
    /// enclosing element that has a use for that key.
    fn metadata(&mut self, e: &BytesStart) {
        let Some(key) = attribute(e, b"key") else {
            return;
        };
        let Some(value) = attribute(e, b"value") else {
            return;
        };
        match key.trim() {
            "name" => {
                if let Some(Scope::Object { meta, .. }) = self.scopes.last_mut() {
                    meta.name = Some(value);
                }
            }
            "extruder" => {
                if let Some(Scope::Object { meta, .. }) = self.scopes.last_mut() {
                    meta.extruder = value.trim().parse().ok();
                }
            }
            "plater_id" => {
                if let Some(Scope::Plate { plater_id, .. }) = self.scopes.last_mut() {
                    *plater_id = value.trim().parse().ok();
                }
            }
            "plater_name" => {
                if let Some(Scope::Plate { name, .. }) = self.scopes.last_mut() {
                    *name = value;
                }
            }
            "object_id" => {
                if let Some(Scope::ModelInstance { object_id }) = self.scopes.last_mut() {
                    *object_id = value.trim().parse().ok();
                }
            }
            // `filament_maps`, `locked`, thumbnails, instance ids and the rest
            // are not needed by the reader.
            _ => {}
        }
    }
}

/// The unescaped value of the attribute with the given local name, if any.
fn attribute(element: &BytesStart, local: &[u8]) -> Option<String> {
    for attribute in element.attributes().flatten() {
        if attribute.key.local_name().as_ref() == local {
            return attribute
                .unescape_value()
                .ok()
                .map(|value| value.into_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::threemf::model::Transform;

    fn item(object_id: u32) -> BuildItem {
        BuildItem {
            object_id,
            transform: Transform::identity(),
        }
    }

    #[test]
    fn parses_plates_and_object_metadata_from_a_real_shaped_config() {
        let xml = br#"<?xml version="1.0"?><config>
          <object id="2"><metadata key="name" value="Body"/><metadata key="extruder" value="0"/>
            <part id="1" subtype="normal_part"><metadata key="matrix" value="1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1"/></part>
          </object>
          <plate><metadata key="plater_id" value="1"/><metadata key="plater_name" value="P1"/>
            <model_instance><metadata key="object_id" value="2"/><metadata key="instance_id" value="0"/></model_instance>
          </plate>
          <plate><metadata key="plater_id" value="2"/><metadata key="plater_name" value="P2"/>
            <model_instance><metadata key="object_id" value="4"/><metadata key="instance_id" value="0"/></model_instance>
          </plate>
        </config>"#;
        let s = parse(xml).unwrap();
        assert!(s.present);
        assert_eq!(s.plates.len(), 2);
        assert_eq!(s.plates[0].plater_id, 1);
        assert_eq!(s.plates[0].object_ids, vec![2]);
        assert_eq!(s.objects[&2].name.as_deref(), Some("Body"));
        assert_eq!(s.objects[&2].extruder, Some(0));
        assert_eq!(s.plates[0].name, "P1");
        assert_eq!(s.plates[1].plater_id, 2);
        assert_eq!(s.plates[1].name, "P2");
        assert_eq!(s.plates[1].object_ids, vec![4]);
    }

    #[test]
    fn a_file_without_model_settings_is_one_plate_holding_every_build_item() {
        let s = ModelSettings {
            objects: Default::default(),
            plates: vec![],
            present: false,
        };
        let build = vec![item(1), item(4)];
        let groups = group(&build, &s);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].item_indices, vec![0, 1]);
        assert_eq!(groups[0].plater_id, None);
        assert_eq!(groups[0].name, "plate 1");
    }

    #[test]
    fn build_items_are_grouped_by_the_plate_that_lists_their_object() {
        let xml = br#"<config>
          <plate><metadata key="plater_id" value="1"/><metadata key="plater_name" value="Front"/>
            <model_instance><metadata key="object_id" value="2"/></model_instance>
            <model_instance><metadata key="object_id" value="7"/></model_instance>
          </plate>
          <plate><metadata key="plater_id" value="2"/><metadata key="plater_name" value="Back"/>
            <model_instance><metadata key="object_id" value="4"/></model_instance>
          </plate>
        </config>"#;
        let s = parse(xml).unwrap();
        let build = vec![item(7), item(4), item(2)];
        let groups = group(&build, &s);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].plater_id, Some(1));
        assert_eq!(groups[0].name, "Front");
        assert_eq!(groups[0].item_indices, vec![0, 2]);
        assert_eq!(groups[1].plater_id, Some(2));
        assert_eq!(groups[1].name, "Back");
        assert_eq!(groups[1].item_indices, vec![1]);
    }

    #[test]
    fn a_build_item_missing_from_every_plate_lands_on_the_first_plate() {
        let xml = br#"<config>
          <plate><metadata key="plater_id" value="1"/>
            <model_instance><metadata key="object_id" value="2"/></model_instance>
          </plate>
          <plate><metadata key="plater_id" value="2"/>
            <model_instance><metadata key="object_id" value="4"/></model_instance>
          </plate>
        </config>"#;
        let s = parse(xml).unwrap();
        let build = vec![item(2), item(99), item(4), item(123)];
        let groups = group(&build, &s);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].item_indices, vec![0, 1, 3]);
        assert_eq!(groups[1].item_indices, vec![2]);
        let placed: usize = groups.iter().map(|g| g.item_indices.len()).sum();
        assert_eq!(placed, build.len(), "no build item may be dropped");
    }

    #[test]
    fn a_plate_with_no_matching_build_items_still_yields_a_group() {
        let xml = br#"<config>
          <plate><metadata key="plater_id" value="1"/></plate>
          <plate><metadata key="plater_id" value="2"/>
            <model_instance><metadata key="object_id" value="4"/></model_instance>
          </plate>
        </config>"#;
        let s = parse(xml).unwrap();
        let groups = group(&[item(4)], &s);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].plater_id, Some(1));
        assert!(groups[0].item_indices.is_empty());
        assert_eq!(groups[1].item_indices, vec![0]);
    }

    #[test]
    fn a_config_without_plates_is_not_present_and_groups_everything_on_plate_one() {
        let xml = br#"<config>
          <object id="2"><metadata key="name" value="Body"/></object>
        </config>"#;
        let s = parse(xml).unwrap();
        assert!(!s.present);
        assert_eq!(s.objects[&2].name.as_deref(), Some("Body"));
        let groups = group(&[item(2), item(4)], &s);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].plater_id, None);
        assert_eq!(groups[0].name, "plate 1");
        assert_eq!(groups[0].item_indices, vec![0, 1]);
    }

    #[test]
    fn missing_plate_metadata_defaults_plater_id_to_the_ordinal_and_name_to_empty() {
        let xml = br#"<config>
          <plate><model_instance><metadata key="object_id" value="5"/></model_instance></plate>
          <plate><metadata key="plater_id" value="9"/></plate>
        </config>"#;
        let s = parse(xml).unwrap();
        assert_eq!(
            s.plates[0].plater_id, 1,
            "first plate defaults to ordinal 1"
        );
        assert_eq!(s.plates[0].name, "");
        assert_eq!(s.plates[0].object_ids, vec![5]);
        assert_eq!(s.plates[1].plater_id, 9);
        assert!(s.plates[1].object_ids.is_empty());
    }

    #[test]
    fn non_numeric_plater_id_falls_back_to_the_ordinal_instead_of_failing() {
        let xml = br#"<config>
          <plate><metadata key="plater_id" value="not-a-number"/></plate>
          <plate><metadata key="plater_id" value="not-a-number"/></plate>
        </config>"#;
        let s = parse(xml).unwrap();
        assert_eq!(s.plates.len(), 2);
        assert_eq!(s.plates[0].plater_id, 1);
        assert_eq!(s.plates[1].plater_id, 2);
    }

    #[test]
    fn unparsable_and_incomplete_entries_are_skipped_not_fatal() {
        let xml = br#"<config>
          <object id="not-a-number"><metadata key="name" value="Bad"/></object>
          <object id="2"><metadata key="name" value="Good"/></object>
          <plate>
            <metadata key="plater_id" value="x"/>
            <metadata key="plater_name" value="Kept"/>
            <model_instance><metadata key="instance_id" value="0"/></model_instance>
            <model_instance><metadata key="object_id" value="not-a-number"/></model_instance>
            <model_instance><metadata key="object_id" value="2"/></model_instance>
          </plate>
        </config>"#;
        let s = parse(xml).unwrap();
        assert_eq!(s.objects.len(), 1);
        assert_eq!(s.objects[&2].name.as_deref(), Some("Good"));
        assert_eq!(s.plates.len(), 1);
        assert_eq!(s.plates[0].plater_id, 1);
        assert_eq!(s.plates[0].name, "Kept");
        assert_eq!(s.plates[0].object_ids, vec![2]);
    }

    #[test]
    fn part_metadata_does_not_override_the_object_metadata() {
        let xml = br#"<config>
          <object id="2"><metadata key="name" value="Body"/><metadata key="extruder" value="3"/>
            <part id="1" subtype="normal_part">
              <metadata key="name" value="PartName"/>
              <metadata key="extruder" value="9"/>
            </part>
          </object>
        </config>"#;
        let s = parse(xml).unwrap();
        assert_eq!(s.objects[&2].name.as_deref(), Some("Body"));
        assert_eq!(s.objects[&2].extruder, Some(3));
    }

    #[test]
    fn an_unparsable_extruder_is_ignored_rather_than_failing() {
        let xml = br#"<config>
          <object id="2"><metadata key="name" value="Body"/></object>
          <object id="3"><metadata key="extruder" value="not-a-number"/></object>
          <object id="4"><metadata key="extruder" value="300"/></object>
        </config>"#;
        let s = parse(xml).unwrap();
        assert_eq!(s.objects[&2].extruder, None);
        assert_eq!(s.objects[&3].extruder, None);
        assert_eq!(s.objects[&4].extruder, None);
    }

    #[test]
    fn an_object_listed_on_two_plates_prints_on_the_first() {
        let xml = br#"<config>
          <plate><metadata key="plater_id" value="1"/>
            <model_instance><metadata key="object_id" value="2"/></model_instance>
          </plate>
          <plate><metadata key="plater_id" value="2"/>
            <model_instance><metadata key="object_id" value="2"/></model_instance>
          </plate>
        </config>"#;
        let s = parse(xml).unwrap();
        let groups = group(&[item(2)], &s);
        assert_eq!(groups[0].item_indices, vec![0]);
        assert!(groups[1].item_indices.is_empty());
    }

    #[test]
    fn the_first_object_block_wins_when_an_id_is_duplicated() {
        let xml = br#"<config>
          <object id="2"><metadata key="name" value="First"/></object>
          <object id="2"><metadata key="name" value="Second"/></object>
        </config>"#;
        let s = parse(xml).unwrap();
        assert_eq!(s.objects[&2].name.as_deref(), Some("First"));
    }

    #[test]
    fn the_assemble_block_contributes_no_plate_membership() {
        let xml = br#"<config>
          <plate><metadata key="plater_id" value="1"/>
            <model_instance><metadata key="object_id" value="2"/></model_instance>
          </plate>
          <assemble>
            <assemble_item object_id="4" instance_id="0" transform="1 0 0 0 1 0 0 0 1 0 0 0" offset="0 0 0"/>
          </assemble>
        </config>"#;
        let s = parse(xml).unwrap();
        assert_eq!(s.plates.len(), 1);
        assert_eq!(s.plates[0].object_ids, vec![2]);
        assert!(!s.objects.contains_key(&4));
    }

    #[test]
    fn malformed_config_is_an_error_not_a_panic() {
        assert!(parse(b"<config><plate>").is_err());
        assert!(parse(b"<config><plate></object></config>").is_err());
        assert!(parse(b"").is_err());
        assert!(parse(b"not xml at all").is_err());
    }
}
