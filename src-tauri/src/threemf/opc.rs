//! Read-only OPC package access for 3MF files.
//!
//! A 3MF file is an OPC (ZIP) package. The package's own relationships
//! (`_rels/.rels`) name the main model part; everything else is reached from
//! there through 3MF references. Only the pieces the reader needs are exposed:
//! the model part name, a guarded entry reader, and the entry list.
//!
//! Every read is bounded by a byte limit that is checked against the entry's
//! declared size *before* any buffer is allocated, so a package cannot make
//! the reader allocate gigabytes just by lying in its central directory.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use zip::ZipArchive;

/// Largest single package entry the reader will inflate. A 3MF stores its
/// geometry as one XML part; real Orca/Bambu projects are well under 100 MB.
pub const MAX_ENTRY_BYTES: u64 = 512 * 1024 * 1024;

/// OPC relationship type of a 3D model part.
const MODEL_RELATIONSHIP_SUFFIX: &str = "/3dmodel";

/// Conventional main model part, used when `_rels/.rels` is absent or unusable
/// (the 3MF core specification allows a package without relationships).
const DEFAULT_MODEL_PART: &str = "3D/3dmodel.model";

const PACKAGE_RELATIONSHIPS: &str = "_rels/.rels";

/// An opened 3MF package.
pub struct Package {
    archive: ZipArchive<File>,
    read_limit: u64,
    /// Resolved main model part, cached after the first lookup.
    model_part: Option<String>,
}

impl Package {
    /// Open a package with the production read limit.
    pub fn open(path: &Path) -> Result<Package, String> {
        Self::open_with_limit(path, MAX_ENTRY_BYTES)
    }

    /// Open a package with an explicit per-entry read limit.
    ///
    /// Exists so the size guard is testable without forging ZIP headers.
    pub(crate) fn open_with_limit(path: &Path, limit: u64) -> Result<Package, String> {
        let file =
            File::open(path).map_err(|e| format!("failed to open {}: {e}", path.display()))?;
        let archive = ZipArchive::new(file)
            .map_err(|e| format!("{} is not a readable OPC/ZIP package: {e}", path.display()))?;
        Ok(Package {
            archive,
            read_limit: limit,
            model_part: None,
        })
    }

    /// The package-relative name of the main model part.
    ///
    /// Follows `_rels/.rels` to the relationship whose type is a 3D model and
    /// falls back to `3D/3dmodel.model` when the relationships are missing or
    /// unreadable. The returned name never has a leading `/`.
    pub fn model_part(&mut self) -> String {
        if let Some(part) = &self.model_part {
            return part.clone();
        }
        let part = match self.read(PACKAGE_RELATIONSHIPS) {
            Ok(bytes) => {
                relationship_target(&bytes).unwrap_or_else(|| DEFAULT_MODEL_PART.to_string())
            }
            Err(_) => DEFAULT_MODEL_PART.to_string(),
        };
        self.model_part = Some(part.clone());
        part
    }

    /// Read one entry, refusing entries whose declared size exceeds the limit.
    ///
    /// The declared size is checked first and then used as the bound for the
    /// buffer, so a forged header cannot make this allocate more than the
    /// limit (and an entry that inflates past its declared size is an error
    /// rather than an unbounded allocation).
    pub fn read(&mut self, name: &str) -> Result<Vec<u8>, String> {
        self.read_at_most(name, self.read_limit)
    }

    pub(crate) fn read_at_most(&mut self, name: &str, limit: u64) -> Result<Vec<u8>, String> {
        let mut entry = self
            .archive
            .by_name(name)
            .map_err(|e| format!("package entry {name:?}: {e}"))?;
        let declared = entry.size();
        let limit = limit.min(self.read_limit);
        if declared > limit {
            return Err(format!(
                "package entry {name:?} declares {declared} bytes, above the {} byte read limit",
                limit
            ));
        }

        let mut bytes = Vec::new();
        let mut bounded = (&mut entry).take(declared.saturating_add(1));
        bounded
            .read_to_end(&mut bytes)
            .map_err(|e| format!("failed to read package entry {name:?}: {e}"))?;
        if bytes.len() as u64 > declared {
            return Err(format!(
                "package entry {name:?} is longer than its declared {declared} bytes"
            ));
        }
        Ok(bytes)
    }

    /// Every entry name in the package, in central-directory order.
    pub fn part_names(&self) -> Vec<String> {
        self.archive.file_names().map(str::to_string).collect()
    }
}

/// Normalise an OPC part name to the form used as a `Scene.parts` key:
/// no surrounding whitespace, no leading `/`, no leading `./`.
pub(crate) fn part_key(path: &str) -> String {
    let mut key = path.trim().trim_start_matches('/');
    while let Some(rest) = key.strip_prefix("./") {
        key = rest;
    }
    key.to_string()
}

/// The target of the first model relationship in an OPC relationships part.
fn relationship_target(xml: &[u8]) -> Option<String> {
    let mut reader = Reader::from_reader(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if e.local_name().as_ref() == b"Relationship" {
                    let target = attribute(&e, b"Target");
                    let kind = attribute(&e, b"Type");
                    if let (Some(target), Some(kind)) = (target, kind) {
                        if kind.trim().ends_with(MODEL_RELATIONSHIP_SUFFIX) {
                            let key = part_key(&target);
                            if !key.is_empty() {
                                return Some(key);
                            }
                        }
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
        buf.clear();
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
