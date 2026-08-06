use petgraph::graph::UnGraph;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::mesh::history::{History, OpKind};

/// Bounding box for a mesh
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundingBox {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// A segment (logical region) of the mesh
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub id: u32,
    pub name: String,
    pub color: Option<[u8; 4]>,
    pub face_count: u32,
}

/// One finalized manual (lasso) region, recorded for undo.
///
/// `faces` stores, per affected face, its pre-finalize `(segment_label,
/// face_color)` so that undo restores the exact prior state. Because the prior
/// state may itself be an earlier manual region, LIFO undo of overlapping
/// regions stays consistent.
pub struct ManualRegionSnapshot {
    pub label: u32,
    pub faces: Vec<(u32, u32, [u8; 4])>, // (face_id, prev_label, prev_color)
}

/// Label offset separating manual segments from auto-segment labels.
/// Auto-segment uses labels 0..N; manual labels start at 100_000 so that
/// re-running auto_segment after manual work never collides.
/// Defined here (with `Segment`) and re-exported by `segment::manual`.
pub const MANUAL_SEGMENT_OFFSET: u32 = 100_000;

/// Neutral mid grey applied to faces that have never been painted, and restored
/// by the eraser.
///
/// Iteration 21: this used to be pure white. Once the 3D canvas started
/// following the UI theme (`--bg-canvas` becomes #eef1f5 in light mode) a white
/// model became unreadable on the light canvas.
///
/// Contrast audit across all four canvas/shading combinations (WCAG relative
/// luminance; shaded values are computed in LINEAR space and re-encoded to sRGB,
/// which is what three.js actually does — skipping that step understates the
/// lit-surface brightness by ~40%):
///
/// | combination            | white #ffffff | grey #8a8a8a |
/// |------------------------|---------------|--------------|
/// | flat  + dark  #2a2a2a  | 14.35:1       | 4.16:1       |
/// | flat  + light #eef1f5  |  1.06:1  BAD  | 3.05:1       |
/// | shaded+ light lit face |  1.06:1  BAD  | 3.05:1       |
/// | shaded+ light shadow   |  2.05:1       | 6.09:1       |
/// | shaded+ dark  shadow   |  6.18:1       | 2.08:1       |
///
/// White fails twice (1.06:1 — the lit side of the model literally disappears
/// into the light canvas). Grey's worst case is 2.08:1 on a fully back-lit face
/// in dark mode, which is an intentionally dark region anyway, and in practice
/// the −5,−5,−5 fill light lifts most of those faces above the ambient floor.
/// Grey is therefore the better global compromise.
///
/// NOTE: this value is also what lands in an exported 3MF for untouched faces.
/// No code path treats "is white" as "is unpainted", so changing it is safe
/// (verified by grep over the whole backend, iteration 21).
pub const DEFAULT_FACE_COLOR: [u8; 4] = [138, 138, 138, 255];

/// Main mesh data structure holding geometry, colors, segmentation, and spatial indices
pub struct MeshModel {
    // Geometry
    pub vertices: Vec<[f32; 3]>,
    pub faces: Vec<[u32; 3]>,
    pub normals: Vec<[f32; 3]>,

    // Per-face RGBA colors
    pub face_colors: Vec<[u8; 4]>,

    // Segmentation
    pub segment_labels: Vec<u32>,
    pub segments: HashMap<u32, Segment>,

    // Manual-region (lasso) undo history — LIFO, populated by finalize_manual_region
    //
    // TRANSITIONAL (Gate 0b' step S1): superseded by `history` below, which now
    // records lasso regions as well. Kept only so the pre-existing frontend
    // `manual_region_undo` path keeps working until step S2 switches the UI over;
    // `undo_last_manual_region` pops both stacks in lockstep so they cannot
    // disagree in the meantime. Removed in S2.
    pub manual_region_history: Vec<ManualRegionSnapshot>,

    // Unified undo/redo history covering every colour and label mutation.
    // Single source of truth once S2 lands. See `mesh::history`.
    pub history: History,

    // Spatial acceleration
    pub face_kdtree: kiddo::KdTree<f32, 3>,
    pub vertex_kdtree: kiddo::KdTree<f32, 3>,
    pub face_adjacency: UnGraph<u32, ()>,

    // Metadata
    pub bbox: BoundingBox,
    pub unit: String,
}

/// Serializable mesh data for IPC transfer to frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshDataDto {
    pub vertices: Vec<f32>,
    pub faces: Vec<u32>,
    pub face_colors: Vec<u8>,
    pub segment_labels: Vec<u32>,
    pub bbox: BoundingBox,
    pub face_count: u32,
    pub segments: Vec<Segment>,
}

impl MeshModel {
    /// Create a new empty MeshModel
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(),
            faces: Vec::new(),
            normals: Vec::new(),
            face_colors: Vec::new(),
            segment_labels: Vec::new(),
            segments: HashMap::new(),
            manual_region_history: Vec::new(),
            history: History::new(),
            face_kdtree: kiddo::KdTree::new(),
            vertex_kdtree: kiddo::KdTree::new(),
            face_adjacency: UnGraph::default(),
            bbox: BoundingBox {
                min: [0.0, 0.0, 0.0],
                max: [0.0, 0.0, 0.0],
            },
            unit: "millimeter".to_string(),
        }
    }

    /// Compute face centers for all triangles
    pub fn face_centers(&self) -> Vec<[f32; 3]> {
        self.faces
            .iter()
            .map(|face| {
                let v0 = self.vertices[face[0] as usize];
                let v1 = self.vertices[face[1] as usize];
                let v2 = self.vertices[face[2] as usize];
                [
                    (v0[0] + v1[0] + v2[0]) / 3.0,
                    (v0[1] + v1[1] + v2[1]) / 3.0,
                    (v0[2] + v1[2] + v2[2]) / 3.0,
                ]
            })
            .collect()
    }

    /// Compute the center of a specific face
    pub fn face_center(&self, face_idx: u32) -> [f32; 3] {
        let face = &self.faces[face_idx as usize];
        let v0 = self.vertices[face[0] as usize];
        let v1 = self.vertices[face[1] as usize];
        let v2 = self.vertices[face[2] as usize];
        [
            (v0[0] + v1[0] + v2[0]) / 3.0,
            (v0[1] + v1[1] + v2[1]) / 3.0,
            (v0[2] + v1[2] + v2[2]) / 3.0,
        ]
    }

    /// Unit normal of a face (right-hand rule over its 3 vertices).
    /// Returns `[0,0,0]` for degenerate triangles so callers can skip them.
    pub fn face_normal(&self, face_idx: u32) -> [f32; 3] {
        let face = &self.faces[face_idx as usize];
        let v0 = self.vertices[face[0] as usize];
        let v1 = self.vertices[face[1] as usize];
        let v2 = self.vertices[face[2] as usize];
        let a = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
        let b = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
        let n = [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len < 1e-12 {
            return [0.0, 0.0, 0.0];
        }
        [n[0] / len, n[1] / len, n[2] / len]
    }

    /// Build KD-Tree from face centers
    pub fn build_kdtree(&mut self) {
        self.face_kdtree = kiddo::KdTree::new();
        let centers = self.face_centers();
        for (i, center) in centers.iter().enumerate() {
            self.face_kdtree.add(center, i as u64);
        }
    }

    /// Build KD-Tree from raw vertices (for nearest-vertex snapping in manual seg)
    pub fn build_vertex_kdtree(&mut self) {
        self.vertex_kdtree = kiddo::KdTree::new();
        for (i, v) in self.vertices.iter().enumerate() {
            self.vertex_kdtree.add(v, i as u64);
        }
    }

    /// Nearest vertex to a point. Returns (vertex_index, squared_distance).
    /// Used by manual segmentation to snap clicked points to the mesh surface.
    pub fn nearest_vertex(&self, point: &[f32; 3]) -> Option<(u32, f32)> {
        if self.vertices.is_empty() {
            return None;
        }
        // kiddo 4 `nearest_one` returns NearestNeighbour directly (not Option).
        let nn = self.vertex_kdtree.nearest_one::<kiddo::SquaredEuclidean>(point);
        Some((nn.item as u32, nn.distance))
    }

    /// First face that contains the given vertex (for vertex→face mapping in
    /// geodesic path computation). Scans faces once; cheap for typical meshes.
    pub fn face_of_vertex(&self, vertex: u32) -> Option<u32> {
        for (fi, face) in self.faces.iter().enumerate() {
            if face[0] == vertex || face[1] == vertex || face[2] == vertex {
                return Some(fi as u32);
            }
        }
        None
    }

    /// Build face adjacency graph using edge sharing
    pub fn build_adjacency(&mut self) {
        let mut edge_to_face: HashMap<(u32, u32), Vec<u32>> = HashMap::new();

        for (face_idx, face) in self.faces.iter().enumerate() {
            let edges = [
                (face[0].min(face[1]), face[0].max(face[1])),
                (face[1].min(face[2]), face[1].max(face[2])),
                (face[0].min(face[2]), face[0].max(face[2])),
            ];
            for edge in edges {
                edge_to_face
                    .entry(edge)
                    .or_default()
                    .push(face_idx as u32);
            }
        }

        self.face_adjacency = UnGraph::new_undirected();
        let face_count = self.faces.len() as u32;
        let node_indices: Vec<_> = (0..face_count)
            .map(|i| self.face_adjacency.add_node(i))
            .collect();

        let mut edge_count = 0u32;
        let mut non_manifold = 0u32;
        // REFUTE P1 fix: connect EVERY pair of faces sharing an edge, including
        // non-manifold edges (faces.len() > 2). A surface graph that drops
        // non-manifold edges breaks geodesic paths and bounded flood-fill used by
        // manual closed-loop segmentation. Pairwise-connect all sharers.
        for faces in edge_to_face.values() {
            if faces.len() >= 2 {
                for i in 0..faces.len() {
                    for j in (i + 1)..faces.len() {
                        self.face_adjacency.add_edge(
                            node_indices[faces[i] as usize],
                            node_indices[faces[j] as usize],
                            (),
                        );
                        edge_count += 1;
                    }
                }
                if faces.len() > 2 {
                    non_manifold += 1;
                }
            }
        }

        log::info!(
            "[build_adjacency] unique_edges={}, manifold_edges={}, non_manifold={}, graph_nodes={}, graph_edges={}",
            edge_to_face.len(),
            edge_count,
            non_manifold,
            self.face_adjacency.node_count(),
            self.face_adjacency.edge_count()
        );
    }

    /// Compute face normals
    pub fn compute_normals(&mut self) {
        self.normals = self
            .faces
            .iter()
            .map(|face| {
                let v0 = self.vertices[face[0] as usize];
                let v1 = self.vertices[face[1] as usize];
                let v2 = self.vertices[face[2] as usize];
                let e1 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
                let e2 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
                let n = [
                    e1[1] * e2[2] - e1[2] * e2[1],
                    e1[2] * e2[0] - e1[0] * e2[2],
                    e1[0] * e2[1] - e1[1] * e2[0],
                ];
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                if len > 1e-10 {
                    [n[0] / len, n[1] / len, n[2] / len]
                } else {
                    [0.0, 0.0, 1.0]
                }
            })
            .collect();
    }

    /// Compute bounding box
    pub fn compute_bbox(&mut self) {
        if self.vertices.is_empty() {
            return;
        }
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for v in &self.vertices {
            for i in 0..3 {
                min[i] = min[i].min(v[i]);
                max[i] = max[i].max(v[i]);
            }
        }
        self.bbox = BoundingBox { min, max };
    }

    /// Initialize every face to `DEFAULT_FACE_COLOR` (neutral mid grey).
    pub fn init_default_colors(&mut self) {
        self.face_colors = vec![DEFAULT_FACE_COLOR; self.faces.len()];
    }

    /// Rebuild `self.segments` (segment metadata) from current per-face labels.
    ///
    /// Single source of truth for segment naming/coloring, used by the
    /// segmentation commands and by manual-region undo so the metadata always
    /// reflects `segment_labels`.
    pub fn rebuild_segments(&mut self) {
        let mut label_counts: HashMap<u32, u32> = HashMap::new();
        for &label in &self.segment_labels {
            *label_counts.entry(label).or_insert(0) += 1;
        }
        let mut segments: Vec<Segment> = label_counts
            .iter()
            .map(|(&label, &count)| {
                let color_seed = label.wrapping_mul(2654435761) >> 24;
                Segment {
                    id: label,
                    name: format!(
                        "Region {}",
                        if label >= MANUAL_SEGMENT_OFFSET {
                            label - MANUAL_SEGMENT_OFFSET + 1
                        } else {
                            label + 1
                        }
                    ),
                    color: if label >= MANUAL_SEGMENT_OFFSET {
                        Some([
                            ((color_seed * 73) % 200 + 55) as u8,
                            ((color_seed * 151) % 200 + 55) as u8,
                            ((color_seed * 223) % 200 + 55) as u8,
                            255,
                        ])
                    } else {
                        None
                    },
                    face_count: count,
                }
            })
            .collect();
        segments.sort_by_key(|s| s.id);
        self.segments = segments.iter().map(|s| (s.id, s.clone())).collect();
    }

    /// Restore a previously snapshotted paint state (full per-face colors +
    /// labels) and rebuild segment metadata. Lengths must match `faces.len()`.
    ///
    /// Single entry point for paint undo/redo: the frontend ships a snapshot
    /// taken before a stroke; calling this reverts `face_colors` / `segment_labels`
    /// and lets `rebuild_segments` drop any segment that the reverted stroke had
    /// just created (its faces fall to 0 → not in `label_counts`).
    pub fn restore_paint_state(
        &mut self,
        face_colors: &[[u8; 4]],
        segment_labels: &[u32],
    ) {
        assert_eq!(face_colors.len(), self.faces.len());
        assert_eq!(segment_labels.len(), self.faces.len());
        self.face_colors = face_colors.to_vec();
        self.segment_labels = segment_labels.to_vec();
        // Wholesale replacement invalidates every recorded diff: an entry holds
        // "face 12 used to be red" against a buffer that no longer exists, and
        // applying it would write red onto whatever the snapshot put there.
        self.history.clear();
        self.rebuild_segments();
    }

    /// Overwrite face colours, recording the previous values so the change can
    /// be undone.
    ///
    /// `updates` carries the *new* colour per face. Callers that derive the new
    /// colour from the old one (the brush blends against it) may read
    /// `face_colors` freely while building `updates`: nothing is written until
    /// the whole batch is recorded, so every read sees the pre-operation state.
    ///
    /// `stroke_id` groups the hundreds of IPC calls a single drag produces into
    /// one undo level. Pass `None` for one-shot operations.
    pub fn apply_paint(&mut self, stroke_id: Option<u64>, updates: &[(u32, [u8; 4])]) {
        let prev: Vec<(u32, [u8; 4])> = updates
            .iter()
            .map(|&(face, _)| (face, self.face_colors[face as usize]))
            .collect();
        self.history.record(OpKind::Paint, stroke_id, &prev, &[]);
        for &(face, color) in updates {
            self.face_colors[face as usize] = color;
        }
    }

    /// Overwrite one face's colour and segment label together, recording both.
    pub fn apply_segment_paint(
        &mut self,
        stroke_id: Option<u64>,
        face: u32,
        label: u32,
        color: [u8; 4],
    ) {
        let i = face as usize;
        self.history.record(
            OpKind::SegmentPaint,
            stroke_id,
            &[(face, self.face_colors[i])],
            &[(face, self.segment_labels[i])],
        );
        self.face_colors[i] = color;
        self.segment_labels[i] = label;
    }

    /// Convert to DTO for IPC transfer
    pub fn to_dto(&self) -> MeshDataDto {
        let vertices: Vec<f32> = self.vertices.iter().flat_map(|v| v.iter().copied()).collect();
        let faces: Vec<u32> = self.faces.iter().flat_map(|f| f.iter().copied()).collect();
        let face_colors: Vec<u8> = self
            .face_colors
            .iter()
            .flat_map(|c| c.iter().copied())
            .collect();
        let segments: Vec<Segment> = self.segments.values().cloned().collect();

        MeshDataDto {
            vertices,
            faces,
            face_colors,
            segment_labels: self.segment_labels.clone(),
            bbox: self.bbox.clone(),
            face_count: self.faces.len() as u32,
            segments,
        }
    }

    /// Query faces within a radius from a point using KD-Tree
    pub fn faces_within_radius(&self, point: &[f32; 3], radius: f32) -> Vec<u32> {
        let results = self
            .face_kdtree
            .within_unsorted::<kiddo::SquaredEuclidean>(point, radius * radius);
        results.iter().map(|item| item.item as u32).collect()
    }
}
