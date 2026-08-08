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

    /// Monotonic high-water mark for manual-segment label allocation.
    ///
    /// Both allocation sites used to derive the next label from
    /// `max(live labels) + 1`, which silently *recycles* numbers: a manual
    /// region that gets fully painted over (or undone) stops appearing in
    /// `segment_labels`, so the next allocation hands out the number it just
    /// vacated. `mesh::history` already documents the consequence — "after a
    /// manual region is undone the next one reuses the same label number, so a
    /// stale `prev_label` would not even be detectably wrong".
    ///
    /// Recycling turns any per-label side table into a resurrection hazard: a
    /// user-supplied name attached to label 100_001 would silently reattach
    /// itself to a brand-new, unrelated region that happened to be issued the
    /// same number. Handing out a fresh number every time costs nothing (u32
    /// exhaustion is unreachable) and makes label identity mean what every
    /// caller already assumes it means.
    pub next_manual_label: u32,

    // Unified undo/redo history covering every colour and label mutation:
    // brush strokes, fills, the eraser, and lasso regions all record here.
    // Sole source of truth for Ctrl+Z since S2. See `mesh::history`.
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
            next_manual_label: MANUAL_SEGMENT_OFFSET,
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

    /// kiddo's KdTree panics ("Too many items with the same position on one
    /// axis") whenever >BUCKET_SIZE points share an IDENTICAL coordinate on the
    /// split axis. That is exactly the case for meshes with many near-coplanar
    /// face centers / vertices — e.g. a UV sphere, where every triangle in a
    /// latitude ring shares the same Z. load_stl built such a tree on Sphere.stl
    /// and the app hard-crashed ("闪退") on import.
    ///
    /// We break exact ties by perturbing each inserted point with a tiny,
    /// deterministic, index-derived offset. Magnitude (~1e-4) is ~1e-6 relative
    /// to model-scale coordinates (≈1e2) and ~1e3× below any paint/spray radius,
    /// so the brush radius queries are unaffected; it only makes points distinct
    /// enough for kiddo to always find a split. Robust at ANY mesh resolution
    /// (unlike bumping BUCKET_SIZE, which a denser sphere would still exceed).
    fn kd_point(mut p: [f32; 3], salt: u64) -> [f32; 3] {
        let h = salt.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let fx = (((h & 0xFFFF) as f32) / 0xFFFF as f32 - 0.5) * 2.0e-4;
        let fy = ((((h >> 16) & 0xFFFF) as f32) / 0xFFFF as f32 - 0.5) * 2.0e-4;
        let fz = ((((h >> 32) & 0xFFFF) as f32) / 0xFFFF as f32 - 0.5) * 2.0e-4;
        p[0] += fx;
        p[1] += fy;
        p[2] += fz;
        p
    }

    /// Build KD-Tree from face centers
    pub fn build_kdtree(&mut self) {
        self.face_kdtree = kiddo::KdTree::new();
        let centers = self.face_centers();
        for (i, center) in centers.iter().enumerate() {
            self.face_kdtree.add(&Self::kd_point(*center, i as u64), i as u64);
        }
    }

    /// Build KD-Tree from raw vertices (for nearest-vertex snapping in manual seg)
    pub fn build_vertex_kdtree(&mut self) {
        self.vertex_kdtree = kiddo::KdTree::new();
        for (i, v) in self.vertices.iter().enumerate() {
            self.vertex_kdtree.add(&Self::kd_point(*v, i as u64), i as u64);
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
            .map(|(&label, &count)| Segment {
                id: label,
                name: format!(
                    "Region {}",
                    if label >= MANUAL_SEGMENT_OFFSET {
                        label - MANUAL_SEGMENT_OFFSET + 1
                    } else {
                        label + 1
                    }
                ),
                color: (label >= MANUAL_SEGMENT_OFFSET).then(|| Self::manual_label_color(label)),
                face_count: count,
            })
            .collect();
        segments.sort_by_key(|s| s.id);
        self.segments = segments.iter().map(|s| (s.id, s.clone())).collect();
    }

    /// Reserve a manual-segment label that has never been issued for this mesh.
    ///
    /// Single allocation point for the manual namespace; `segment::manual` and
    /// `commands::segment` each used to carry their own copy of
    /// `max(MANUAL_SEGMENT_OFFSET, max_live_label + 1)`.
    ///
    /// The high-water mark alone is not sufficient: labels can also arrive from
    /// outside this allocator (a loaded project, a future importer), so the
    /// floor is taken against the live maximum as well. Whichever is larger
    /// wins, and the mark then advances past it — so the result is unique both
    /// against everything currently on the mesh and against everything this
    /// mesh has ever handed out.
    pub fn alloc_manual_label(&mut self) -> u32 {
        let live_max = self
            .segment_labels
            .iter()
            .copied()
            .filter(|&l| l >= MANUAL_SEGMENT_OFFSET)
            .max();
        let floor = match live_max {
            Some(m) => m.saturating_add(1),
            None => MANUAL_SEGMENT_OFFSET,
        };
        let label = floor.max(self.next_manual_label);
        self.next_manual_label = label.saturating_add(1);
        label
    }

    /// Deterministic per-label colour for manual segments.
    ///
    /// Derived from the label so a region keeps its colour across rebuilds
    /// without storing it. Three call sites (lasso finalize, segment brush,
    /// metadata rebuild) previously inlined this same hash; they must agree,
    /// because the brush writes it into `face_colors` while `rebuild_segments`
    /// reports it as `Segment.color`, and a mismatch shows up as a region whose
    /// swatch does not match the model.
    pub fn manual_label_color(label: u32) -> [u8; 4] {
        let seed = label.wrapping_mul(2654435761) >> 24;
        [
            ((seed * 73) % 200 + 55) as u8,
            ((seed * 151) % 200 + 55) as u8,
            ((seed * 223) % 200 + 55) as u8,
            255,
        ]
    }

    /// Segment metadata as a list ordered by id.
    ///
    /// `segments` is a `HashMap`, so iterating it yields a different order on
    /// every call. Anything that crosses IPC must go through here: the frontend
    /// renders the region list straight from this vector and keys React rows by
    /// index in places, so an unstable order makes the panel reshuffle itself
    /// after every undo/redo even when nothing changed.
    pub fn sorted_segments(&self) -> Vec<Segment> {
        let mut segments: Vec<Segment> = self.segments.values().cloned().collect();
        segments.sort_by_key(|s| s.id);
        segments
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
        let segments = self.sorted_segments();

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Label allocation only reads `segment_labels`, so a bare model with a
    /// label buffer is a complete fixture — no geometry required.
    fn labelled(labels: &[u32]) -> MeshModel {
        let mut m = MeshModel::new();
        m.segment_labels = labels.to_vec();
        m
    }

    #[test]
    fn first_manual_label_is_the_namespace_offset() {
        let mut m = labelled(&[0, 0, 1, 1]);
        assert_eq!(m.alloc_manual_label(), MANUAL_SEGMENT_OFFSET);
    }

    #[test]
    fn manual_labels_are_handed_out_in_sequence() {
        let mut m = labelled(&[0; 4]);
        let a = m.alloc_manual_label();
        m.segment_labels[0] = a;
        let b = m.alloc_manual_label();
        assert_eq!((a, b), (MANUAL_SEGMENT_OFFSET, MANUAL_SEGMENT_OFFSET + 1));
    }

    /// Regression: the allocator used to be `max(live labels) + 1`, so a region
    /// that was undone or fully painted over released its number back into the
    /// pool and the next region was issued the *same* label. Any per-label side
    /// table (a user-supplied name, most obviously) would then reattach itself
    /// to an unrelated region. Retiring a label must not make it reusable.
    #[test]
    fn a_retired_manual_label_is_never_issued_again() {
        let mut m = labelled(&[0; 4]);
        let first = m.alloc_manual_label();
        m.segment_labels[0] = first;

        // The region is painted over: `first` no longer appears anywhere.
        m.segment_labels[0] = 0;
        assert!(!m.segment_labels.contains(&first));

        let second = m.alloc_manual_label();
        assert_ne!(
            second, first,
            "allocator recycled a retired label ({first}) — side tables keyed by \
             label would resurrect onto an unrelated region"
        );
    }

    /// The high-water mark is not the only constraint: labels can be present
    /// without this allocator having issued them (a loaded project, a future
    /// importer). The live maximum has to be respected too, or the "fresh"
    /// label would collide with an existing region.
    #[test]
    fn labels_from_outside_the_allocator_still_raise_the_floor() {
        let mut m = labelled(&[0, MANUAL_SEGMENT_OFFSET + 40]);
        assert_eq!(m.alloc_manual_label(), MANUAL_SEGMENT_OFFSET + 41);
    }

    /// The brush writes `manual_label_color` into `face_colors` while
    /// `rebuild_segments` reports it as `Segment.color`; if the two ever
    /// disagreed the region list swatch would not match the model.
    #[test]
    fn rebuilt_metadata_colour_matches_the_painted_colour() {
        let label = MANUAL_SEGMENT_OFFSET + 7;
        let mut m = labelled(&[0, label, label]);
        m.rebuild_segments();

        let seg = m.segments.get(&label).expect("manual segment present");
        assert_eq!(seg.color, Some(MeshModel::manual_label_color(label)));
        assert_eq!(seg.face_count, 2);
        // Auto labels stay uncoloured: the renderer derives their colour itself.
        assert_eq!(m.segments.get(&0).expect("auto segment").color, None);
    }
}
