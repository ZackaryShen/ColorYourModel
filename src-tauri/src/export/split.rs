//! Split-by-colour export spike (0.2.0-P2, requirement 5).
//!
//! Exports the painted model as one file per connected same-colour shell, so
//! slicers WITHOUT a paint/multi-material workflow (Cura, PrusaSlicer) can
//! print each colour as a separate plate item. This is the technical spike
//! the 0.2.0 plan asked for: it deliberately does NOT close holes — each part
//! is the open surface shell of its colour region — and the findings are
//! recorded in `docs/technical/split-export-spike.md`.
//!
//! Pipeline: quantize face colors to ≤16 slots (same path as the 3MF
//! extruder-slot writer) → split each slot into connected components
//! (face-adjacency BFS, mirroring `split_connected_components`) → extract
//! each component as a standalone `MeshModel` (vertex remap, original
//! per-face colors preserved) → `export_obj` per part + a manifest.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use petgraph::visit::EdgeRef;

use super::quantize::quantize_face_colors;
use super::obj::export_obj;
use crate::mesh::model::MeshModel;

/// Maximum colour slots, mirroring the 3MF extruder-slot ceiling
/// (`paint_color.rs MAX_EXTRUDER_SLOT`).
const MAX_COLOR_SLOTS: usize = 16;

/// Extract the given faces into a standalone mesh. Vertices are remapped to
/// the used subset (order-preserving); face colors and segment labels travel
/// with their faces; all derived state is rebuilt.
fn extract_submesh(mesh: &MeshModel, faces: &[u32]) -> MeshModel {
    let mut vertex_map: HashMap<u32, u32> = HashMap::new();
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut out_faces: Vec<[u32; 3]> = Vec::with_capacity(faces.len());
    for &f in faces {
        let tri = mesh.faces[f as usize];
        let mut mapped = [0u32; 3];
        for (i, &vi) in tri.iter().enumerate() {
            let next_id = vertex_map.len() as u32;
            let id = *vertex_map.entry(vi).or_insert(next_id);
            if id == next_id {
                vertices.push(mesh.vertices[vi as usize]);
            }
            mapped[i] = id;
        }
        out_faces.push(mapped);
    }

    let mut sub = MeshModel::new();
    sub.vertices = vertices;
    sub.faces = out_faces;
    sub.face_colors = faces.iter().map(|&f| mesh.face_colors[f as usize]).collect();
    sub.segment_labels = faces.iter().map(|&f| mesh.segment_labels[f as usize]).collect();
    sub.unit = mesh.unit.clone();
    sub.compute_normals();
    sub.compute_bbox();
    sub.rebuild_segments();
    sub.build_kdtree();
    sub.build_vertex_kdtree();
    sub.build_adjacency();
    sub
}

/// Split by quantized colour into connected same-colour shells and export one
/// OBJ per shell into `dir`. Returns the written part paths (manifest at
/// `dir/manifest.txt`). Parts are named `part-SS-KK.obj` where SS is the
/// 1-based colour slot (sorted by descending face count, mirroring the 3MF
/// slot order) and KK is the shell index within the slot — deterministic
/// across runs because the BFS starts from the lowest face index.
pub fn export_split_by_color(
    mesh: &MeshModel,
    dir: &Path,
    progress: &dyn Fn(f32, &str),
) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("split: create dir: {e}"))?;
    let quantized = quantize_face_colors(&mesh.face_colors, MAX_COLOR_SLOTS);

    // Group faces by (slot) preserving the palette's descending-count order.
    let mut slot_faces: HashMap<u8, Vec<u32>> = HashMap::new();
    for (fi, &slot) in quantized.face_slots.iter().enumerate() {
        slot_faces.entry(slot).or_default().push(fi as u32);
    }
    let mut slots: Vec<u8> = slot_faces.keys().copied().collect();
    slots.sort_unstable();

    let mut parts: Vec<PathBuf> = Vec::new();
    let mut manifest: Vec<String> = Vec::new();
    manifest.push("# split-by-colour export — open surface shells (no hole closing)".to_string());
    manifest.push("# file | slot | rgb | faces".to_string());

    let total = slots.len();
    for (done, &slot) in slots.iter().enumerate() {
        let faces = &slot_faces[&slot];
        // Connected components within this slot (slot id as the group label).
        let mut comp_of: Vec<u32> = vec![u32::MAX; mesh.faces.len()];
        let mut shells: Vec<Vec<u32>> = Vec::new();
        for &start in faces {
            let si = start as usize;
            if comp_of[si] != u32::MAX {
                continue;
            }
            let id = shells.len() as u32;
            let mut q = vec![start];
            comp_of[si] = id;
            let mut shell = vec![start];
            while let Some(cur) = q.pop() {
                let node = petgraph::graph::NodeIndex::new(cur as usize);
                for edge in mesh.face_adjacency.edges(node) {
                    let nb = if edge.source() == node {
                        edge.target()
                    } else {
                        edge.source()
                    };
                    let ni = nb.index();
                    if quantized.face_slots[ni] != slot || comp_of[ni] != u32::MAX {
                        continue;
                    }
                    comp_of[ni] = id;
                    q.push(ni as u32);
                    shell.push(ni as u32);
                }
            }
            shells.push(shell);
        }

        for (k, shell) in shells.iter().enumerate() {
            let sub = extract_submesh(mesh, shell);
            let name = format!("part-{:02}-{:02}.obj", slot, k);
            let out_path = dir.join(&name);
            export_obj(&sub, &out_path, &|_, _| {})
                .map_err(|e| format!("split: export {name}: {e}"))?;
            let rgb = quantized.palette[(slot - 1) as usize];
            manifest.push(format!(
                "{} | slot {} | rgb({},{},{}) | {} faces",
                name,
                slot,
                rgb[0],
                rgb[1],
                rgb[2],
                shell.len()
            ));
            parts.push(out_path);
        }
        progress((done + 1) as f32 / total as f32, &format!("slot {slot}"));
    }

    let manifest_path = dir.join("manifest.txt");
    std::fs::write(&manifest_path, manifest.join("\n") + "\n")
        .map_err(|e| format!("split: write manifest: {e}"))?;
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::model::MANUAL_SEGMENT_OFFSET;

    /// Four separated 10mm cubes (12 triangles each, properly connected),
    /// painted red / green / blue / red — exercises slot grouping (red
    /// appears on two disconnected shells) and shell splitting.
    fn three_colour_model() -> MeshModel {
        let mut m = MeshModel::new();
        let mut push_cube = |ox: f32, oy: f32| {
            let base = m.vertices.len() as u32;
            let s = 10.0f32;
            let mut corner = |dx: f32, dy: f32, dz: f32| {
                m.vertices.push([ox + dx * s, oy + dy * s, dz * s]);
            };
            // 8 corners: bottom (z=0) then top (z=10)
            corner(0.0, 0.0, 0.0);
            corner(1.0, 0.0, 0.0);
            corner(1.0, 1.0, 0.0);
            corner(0.0, 1.0, 0.0);
            corner(0.0, 0.0, 1.0);
            corner(1.0, 0.0, 1.0);
            corner(1.0, 1.0, 1.0);
            corner(0.0, 1.0, 1.0);
            // 12 triangles (bottom, top, 4 sides), all connected.
            let tris = [
                [0, 2, 1], [0, 3, 2], // bottom
                [4, 5, 6], [4, 6, 7], // top
                [0, 1, 5], [0, 5, 4], // front
                [1, 2, 6], [1, 6, 5], // right
                [2, 3, 7], [2, 7, 6], // back
                [3, 0, 4], [3, 4, 7], // left
            ];
            for t in tris {
                m.faces.push([base + t[0], base + t[1], base + t[2]]);
            }
        };
        push_cube(0.0, 0.0);
        push_cube(15.0, 0.0);
        push_cube(0.0, 15.0);
        push_cube(15.0, 15.0);
        m.compute_normals();
        m.compute_bbox();
        m.init_default_colors();
        m.segment_labels = vec![0; m.faces.len()];
        m.build_kdtree();
        m.build_vertex_kdtree();
        m.build_adjacency();
        m.next_manual_label = MANUAL_SEGMENT_OFFSET;
        // 12 faces per cube: red / green / blue / red.
        for i in 0..12 {
            m.face_colors[i] = [255, 0, 0, 255];
        }
        for i in 12..24 {
            m.face_colors[i] = [0, 255, 0, 255];
        }
        for i in 24..36 {
            m.face_colors[i] = [0, 0, 255, 255];
        }
        for i in 36..48 {
            m.face_colors[i] = [255, 0, 0, 255];
        }
        m
    }

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("cym_split_{}_{}", std::process::id(), name));
        p
    }

    #[test]
    fn splits_three_colours_into_separate_valid_parts() {
        let model = three_colour_model();
        let dir = tmp_dir("three");
        let parts = export_split_by_color(&model, &dir, &|_, _| {}).expect("split");
        // red × 2 (disconnected cubes) + green + blue = 4 shells.
        assert_eq!(parts.len(), 4, "shells: {:?}", parts);
        for p in &parts {
            assert!(p.exists(), "{:?} missing", p);
            assert!(std::fs::metadata(p).unwrap().len() > 0);
        }
        let manifest = std::fs::read_to_string(dir.join("manifest.txt")).unwrap();
        assert!(manifest.contains("slot 1"));
        assert!(manifest.contains("slot 3"));
        // The two red cubes are disconnected → at least 2 red parts... with
        // 3 slots total the part count is 4 (red×2 + green + blue).
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn extracted_parts_have_remapped_consistent_geometry() {
        let model = three_colour_model();
        let dir = tmp_dir("remap");
        let parts = export_split_by_color(&model, &dir, &|_, _| {}).expect("split");
        // Every part must load through the STL/OBJ loader-independent check:
        // vertices referenced by its faces exist (re-export via export_obj
        // again would trivially pass; instead assert index bounds directly by
        // re-reading the OBJ text).
        for p in &parts {
            let text = std::fs::read_to_string(p).unwrap();
            let mut v_count = 0usize;
            for line in text.lines() {
                if let Some(rest) = line.strip_prefix("v ") {
                    v_count += 1;
                    let _ = rest;
                }
                if let Some(rest) = line.strip_prefix("f ") {
                    for tok in rest.split_whitespace() {
                        let vi: usize =
                            tok.split('/').next().unwrap().parse().expect("face index");
                        assert!(vi >= 1 && vi <= v_count || v_count >= vi, "index {} > v {}", vi, v_count);
                    }
                }
            }
            assert!(v_count > 0);
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
