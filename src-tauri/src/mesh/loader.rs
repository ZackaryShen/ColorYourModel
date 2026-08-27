use crate::mesh::model::MeshModel;
use std::collections::HashMap;
use std::path::Path;

/// Progress callback type: (fraction 0.0..1.0, stage description)
pub type ProgressFn = dyn Fn(f32, &str) + Send + Sync;

/// Load an STL file and build a MeshModel, reporting progress via callback
pub fn load_stl(path: &Path, on_progress: &ProgressFn) -> Result<MeshModel, String> {
    on_progress(0.05, "Parsing STL file...");
    log::info!("[loader] Loading STL: {:?}", path);

    let mut reader =
        std::fs::File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
    let stl_mesh =
        stl_io::read_stl(&mut reader).map_err(|e| format!("Failed to parse STL: {}", e))?;

    let tri_count = stl_mesh.faces.len();
    let raw_vert_count = stl_mesh.vertices.len();
    log::info!(
        "[loader] STL parsed: {} triangles, {} raw vertices",
        tri_count, raw_vert_count
    );
    on_progress(
        0.20,
        &format!("Parsed {} triangles, {} vertices", tri_count, raw_vert_count),
    );

    on_progress(0.25, "Merging duplicate vertices...");
    let mut model = MeshModel::new();

    // Merge duplicate vertices using a HashMap keyed by bit-pattern of f32 coords
    let mut vertex_map: HashMap<[u32; 3], u32> = HashMap::new();
    let mut merged_vertices: Vec<[f32; 3]> = Vec::new();
    let mut faces: Vec<[u32; 3]> = Vec::new();

    for triangle in &stl_mesh.faces {
        let mut face_indices = [0u32; 3];
        for (i, &vert_idx) in triangle.vertices.iter().enumerate() {
            let vert = stl_mesh.vertices[vert_idx];
            let key = [
                vert[0].to_bits(),
                vert[1].to_bits(),
                vert[2].to_bits(),
            ];
            if let Some(&idx) = vertex_map.get(&key) {
                face_indices[i] = idx;
            } else {
                let idx = merged_vertices.len() as u32;
                vertex_map.insert(key, idx);
                merged_vertices.push([vert[0], vert[1], vert[2]]);
                face_indices[i] = idx;
            }
        }
        faces.push(face_indices);
    }

    let vert_count = merged_vertices.len();
    on_progress(
        0.45,
        &format!("Merged to {} unique vertices", vert_count),
    );

    model.vertices = merged_vertices;
    model.faces = faces;

    // Post-process
    on_progress(0.50, "Computing face normals...");
    model.compute_normals();

    on_progress(0.55, "Computing bounding box...");
    model.compute_bbox();

    on_progress(0.60, "Initializing default colors...");
    model.init_default_colors();
    // Initialize per-face segment labels to a valid length (all 0 = unsegmented
    // background). Without this the vector is empty and any paint/segment/undo
    // op that indexes `segment_labels[face_id]` panics (index out of bounds) and
    // poisons the mesh Mutex — the real root cause of "分区笔点一下崩溃" and
    // "undo 崩溃" (iteration 18, REFUTE B1/B4). `restore_paint_state`'s
    // `assert_eq!(segment_labels.len(), faces.len())` also requires this length.
    model.segment_labels = vec![0u32; model.faces.len()];

    on_progress(0.65, "Building spatial index (KD-Tree)...");
    model.build_kdtree();
    model.build_vertex_kdtree();

    on_progress(0.75, "Building adjacency graph...");
    model.build_adjacency();

    on_progress(0.95, "Finalizing...");

    log::info!(
        "[loader] Mesh ready: {} verts, {} faces, bbox=[{:.1},{:.1},{:.1}]→[{:.1},{:.1},{:.1}]",
        model.vertices.len(),
        model.faces.len(),
        model.bbox.min[0], model.bbox.min[1], model.bbox.min[2],
        model.bbox.max[0], model.bbox.max[1], model.bbox.max[2]
    );

    Ok(model)
}
