use petgraph::graph::UnGraph;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

    /// Initialize default white colors for all faces
    pub fn init_default_colors(&mut self) {
        self.face_colors = vec![[255, 255, 255, 255]; self.faces.len()];
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
