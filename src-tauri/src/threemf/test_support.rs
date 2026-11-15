#![cfg(test)]
//! Test-only helpers shared by the 3MF reader tests.
//!
//! Task 9 extends this file with fixture builders; for now it only provides
//! the real-file scanner used by the `#[ignore]`-d paint tests.

use std::io::Read;
use std::path::Path;

/// Every `paint_color="..."` value found in every `.model` part of a 3MF, in
/// file order and with duplicates kept.
///
/// A plain byte scan is enough here: the tests only need the distinct values,
/// not a real XML parse. Panics on I/O errors because this is test support.
pub fn paint_colors_in(path: &Path) -> Vec<String> {
    let file = std::fs::File::open(path)
        .unwrap_or_else(|e| panic!("open {}: {e}", path.display()));
    let mut zip = zip::ZipArchive::new(file)
        .unwrap_or_else(|e| panic!("{} is not a zip archive: {e}", path.display()));

    let mut values = Vec::new();
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .unwrap_or_else(|e| panic!("zip entry {index}: {e}"));
        let name = entry.name().to_string();
        if !name.ends_with(".model") {
            continue;
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .unwrap_or_else(|e| panic!("read {name}: {e}"));
        values.extend(scan_paint_colors(&bytes, &name));
    }
    values
}

const NEEDLE: &[u8] = b"paint_color=\"";

fn scan_paint_colors(bytes: &[u8], part: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut cursor = 0;
    while cursor + NEEDLE.len() <= bytes.len() {
        if &bytes[cursor..cursor + NEEDLE.len()] == NEEDLE {
            let start = cursor + NEEDLE.len();
            let Some(len) = bytes[start..].iter().position(|&b| b == b'"') else {
                break;
            };
            let value = std::str::from_utf8(&bytes[start..start + len])
                .unwrap_or_else(|e| panic!("paint_color in {part} is not UTF-8: {e}"));
            values.push(value.to_string());
            cursor = start + len + 1;
        } else {
            cursor += 1;
        }
    }
    values
}
