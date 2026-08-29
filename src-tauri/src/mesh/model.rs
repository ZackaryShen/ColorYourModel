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

/// Ceiling on a user-supplied region name, in characters (not bytes).
///
/// The panel row is one line; anything past this is already elided visually,
/// and an unbounded string crosses IPC on every segment rebuild.
pub const MAX_SEGMENT_NAME_CHARS: usize = 64;

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

    /// User-supplied region names, keyed by segment label.
    ///
    /// `segments` is derived state — every label rewrite throws it away and
    /// rebuilds it from `segment_labels` — so a name stored on `Segment` would
    /// survive exactly until the next lasso stroke. It also would not survive
    /// the trip through the frontend: `appStore` re-projects segments into
    /// `{id, name, color, faceCount}` in three places, silently dropping any
    /// field added to the DTO. Keeping names in their own label-keyed table on
    /// the model is the only place both problems go away at once.
    ///
    /// Entries are deliberately **not** pruned when a label stops appearing in
    /// `segment_labels`. Undoing a lasso region removes its label; redoing it
    /// brings the same label back (labels are never recycled, see
    /// `next_manual_label`), and the user expects their name to come back with
    /// it. Auto labels are the exception and are purged by `run_segmentation` —
    /// a re-run renumbers regions from scratch, so keeping "Left arm" on label 3
    /// would reattach it to whatever the algorithm happens to call 3 next time.
    pub segment_names: HashMap<u32, String>,

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
            segment_names: HashMap::new(),
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
    pub(crate) fn kd_point(mut p: [f32; 3], salt: u64) -> [f32; 3] {
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
        let t_edge_map = std::time::Instant::now();
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
        let t_edge_map_done = t_edge_map.elapsed();

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
            "[build_adjacency] edge_map={:?} graph_build={:?} unique_edges={}, manifold_edges={}, non_manifold={}, graph_nodes={}, graph_edges={}",
            t_edge_map_done,
            t_edge_map.elapsed(),
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
                name: self
                    .segment_names
                    .get(&label)
                    .cloned()
                    .unwrap_or_else(|| Self::default_segment_name(label)),
                color: (label >= MANUAL_SEGMENT_OFFSET).then(|| Self::manual_label_color(label)),
                face_count: count,
            })
            .collect();
        segments.sort_by_key(|s| s.id);
        self.segments = segments.iter().map(|s| (s.id, s.clone())).collect();
    }

    /// The name a region carries until the user gives it one.
    ///
    /// Manual labels are numbered from their namespace offset rather than their
    /// raw value, so the first hand-drawn region reads "Region 1" and not
    /// "Region 100001".
    pub fn default_segment_name(label: u32) -> String {
        let ordinal = if label >= MANUAL_SEGMENT_OFFSET {
            label - MANUAL_SEGMENT_OFFSET + 1
        } else {
            label + 1
        };
        format!("Region {}", ordinal)
    }

    /// Attach a user-supplied name to a region, or clear it back to the default.
    ///
    /// Rejects labels that are not currently on the mesh. A rename targeting a
    /// region that an undo just dissolved is a frontend bug, and silently
    /// creating an orphan entry would make it show up much later as a name
    /// appearing on an unrelated region.
    ///
    /// An empty (or whitespace-only) name removes the override instead of
    /// storing a blank row: a blank label in the panel is indistinguishable
    /// from a rendering failure, and "clear it back to Region N" is what the
    /// user means when they delete the text and press Enter.
    pub fn rename_segment(&mut self, label: u32, name: &str) -> Result<(), String> {
        if !self.segments.contains_key(&label) {
            return Err(format!("Segment {} does not exist", label));
        }
        let trimmed = name.trim();
        if trimmed.is_empty() {
            self.segment_names.remove(&label);
        } else {
            // Truncate on char boundaries — a byte slice would panic on the
            // first multi-byte character, and region names are the one place a
            // user is guaranteed to type CJK.
            let capped: String = trimmed.chars().take(MAX_SEGMENT_NAME_CHARS).collect();
            self.segment_names.insert(label, capped);
        }
        self.rebuild_segments();
        Ok(())
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

    /// Absorb one or more regions into `target`. Returns how many faces moved.
    ///
    /// **Labels only — merging never repaints.** The tempting alternative is to
    /// flood the absorbed faces with the target's colour so the result "looks
    /// merged", but `face_colors` is the buffer the 3MF exporter reads: that
    /// version of merge would throw away the user's actual output to tidy up a
    /// grouping. The segment view already draws the whole merged region in one
    /// colour because it derives colour from the label, which is exactly the
    /// feedback the operation needs.
    ///
    /// Names of the absorbed regions are left in `segment_names` on purpose,
    /// matching the rule the rest of the table follows: undoing the merge
    /// brings those labels back to the same faces, and the user would not
    /// expect one undo to restore the region but not what they called it.
    pub fn merge_segments(&mut self, target: u32, others: &[u32]) -> Result<usize, String> {
        if !self.segments.contains_key(&target) {
            return Err(format!("Segment {} does not exist", target));
        }
        let sources: std::collections::HashSet<u32> =
            others.iter().copied().filter(|&l| l != target).collect();
        if sources.is_empty() {
            return Err("Merge needs at least one region other than the target".to_string());
        }
        if let Some(missing) = sources.iter().find(|l| !self.segments.contains_key(l)) {
            return Err(format!("Segment {} does not exist", missing));
        }

        let prev_labels: Vec<(u32, u32)> = self
            .segment_labels
            .iter()
            .enumerate()
            .filter(|(_, l)| sources.contains(l))
            .map(|(i, &l)| (i as u32, l))
            .collect();
        if prev_labels.is_empty() {
            return Ok(0);
        }

        self.history.record(OpKind::Merge, None, &[], &prev_labels);
        for &(face, _) in &prev_labels {
            self.segment_labels[face as usize] = target;
        }
        self.rebuild_segments();
        Ok(prev_labels.len())
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

    #[test]
    fn a_named_region_reports_its_name_instead_of_the_default() {
        let mut m = labelled(&[0, 0, 1]);
        m.rebuild_segments();
        assert_eq!(m.segments[&1].name, "Region 2");

        m.rename_segment(1, "Left arm").unwrap();
        assert_eq!(m.segments[&1].name, "Left arm");
        assert_eq!(m.segments[&0].name, "Region 1", "siblings keep the default");
    }

    /// The name has to outlive the metadata it is displayed on: every lasso
    /// stroke throws `segments` away and rebuilds it from the labels.
    #[test]
    fn a_name_survives_a_metadata_rebuild() {
        let label = MANUAL_SEGMENT_OFFSET;
        let mut m = labelled(&[label, label, 0]);
        m.rebuild_segments();
        m.rename_segment(label, "Handle").unwrap();

        // Something else repaints part of the mesh and the metadata is rebuilt.
        m.segment_labels[2] = label;
        m.rebuild_segments();

        assert_eq!(m.segments[&label].name, "Handle");
        assert_eq!(m.segments[&label].face_count, 3);
    }

    /// Blank input means "I want the default back", not "store an empty row".
    #[test]
    fn clearing_a_name_restores_the_default() {
        let mut m = labelled(&[0, 0]);
        m.rebuild_segments();
        m.rename_segment(0, "Base").unwrap();
        m.rename_segment(0, "   ").unwrap();

        assert_eq!(m.segments[&0].name, "Region 1");
        assert!(m.segment_names.is_empty(), "no blank override left behind");
    }

    /// Renaming a region that is not on the mesh is a caller bug. Accepting it
    /// would park an orphan entry that resurfaces on a future region.
    #[test]
    fn renaming_a_region_that_does_not_exist_is_rejected() {
        let mut m = labelled(&[0, 0]);
        m.rebuild_segments();
        assert!(m.rename_segment(42, "Ghost").is_err());
        assert!(m.segment_names.is_empty());
    }

    /// The cap counts characters, not bytes — region names are exactly where a
    /// user types CJK, and slicing a `String` by byte index would panic.
    #[test]
    fn an_overlong_name_is_capped_on_a_character_boundary() {
        let mut m = labelled(&[0, 0]);
        m.rebuild_segments();
        let long = "左臂上部结构".repeat(40);
        m.rename_segment(0, &long).unwrap();

        let stored = &m.segments[&0].name;
        assert_eq!(stored.chars().count(), MAX_SEGMENT_NAME_CHARS);
        assert!(long.starts_with(stored.as_str()));
    }

    #[test]
    fn merging_moves_every_face_of_the_absorbed_regions() {
        let mut m = labelled(&[0, 1, 1, 2, 3]);
        m.face_colors = vec![DEFAULT_FACE_COLOR; 5];
        m.rebuild_segments();

        let moved = m.merge_segments(0, &[1, 2]).unwrap();

        assert_eq!(moved, 3);
        assert_eq!(m.segment_labels, vec![0, 0, 0, 0, 3]);
        assert_eq!(m.segments[&0].face_count, 4);
        assert!(!m.segments.contains_key(&1), "absorbed region is gone");
        assert!(m.segments.contains_key(&3), "untouched region survives");
    }

    /// The exporter reads `face_colors`, so a merge that "tidied up" the colour
    /// of the absorbed faces would be destroying the actual print output to fix
    /// a grouping. Segment view already shows the merge, because it colours by
    /// label.
    #[test]
    fn merging_leaves_the_paint_alone() {
        let mut m = labelled(&[0, 1, 1]);
        m.face_colors = vec![[10, 20, 30, 255], [200, 0, 0, 255], [0, 200, 0, 255]];
        m.rebuild_segments();

        m.merge_segments(0, &[1]).unwrap();

        assert_eq!(
            m.face_colors,
            vec![[10, 20, 30, 255], [200, 0, 0, 255], [0, 200, 0, 255]]
        );
    }

    #[test]
    fn a_merge_can_be_undone() {
        let mut m = labelled(&[0, 1, 1, 2]);
        m.face_colors = vec![DEFAULT_FACE_COLOR; 4];
        m.rebuild_segments();
        m.merge_segments(0, &[1]).unwrap();

        let MeshModel {
            history,
            face_colors,
            segment_labels,
            ..
        } = &mut m;
        let out = history.undo(face_colors, segment_labels).expect("undo");

        assert!(out.labels_changed);
        assert_eq!(m.segment_labels, vec![0, 1, 1, 2]);
    }

    /// Undo restores the labels, so the names they carry have to still be there
    /// when they come back.
    #[test]
    fn undoing_a_merge_brings_the_name_back_with_the_region() {
        let mut m = labelled(&[0, 1, 1]);
        m.face_colors = vec![DEFAULT_FACE_COLOR; 3];
        m.rebuild_segments();
        m.rename_segment(1, "Spout").unwrap();
        m.merge_segments(0, &[1]).unwrap();
        assert!(!m.segments.contains_key(&1));

        let MeshModel {
            history,
            face_colors,
            segment_labels,
            ..
        } = &mut m;
        history.undo(face_colors, segment_labels).expect("undo");
        m.rebuild_segments();

        assert_eq!(m.segments[&1].name, "Spout");
    }

    #[test]
    fn merging_rejects_regions_that_do_not_exist() {
        let mut m = labelled(&[0, 1]);
        m.rebuild_segments();

        assert!(m.merge_segments(9, &[1]).is_err(), "unknown target");
        assert!(m.merge_segments(0, &[9]).is_err(), "unknown source");
        assert!(m.merge_segments(0, &[0]).is_err(), "merging into itself");
        assert_eq!(m.segment_labels, vec![0, 1], "a rejected merge changes nothing");
    }
}
