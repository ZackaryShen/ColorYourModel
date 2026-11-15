//! Rebuild of the subdivided mesh described by the `paint_color` trees.
//!
//! This is a port of `TriangleSelector::perform_split`
//! (`src/libslic3r/TriangleSelector.cpp`, OrcaSlicer). The slicer allocates a
//! new midpoint through `triangle_midpoint_or_allocate`, which walks its
//! neighbour table so that two triangles sharing an edge receive the identical
//! vertex index. Here the same guarantee comes from a cache keyed by the vertex
//! index pair `(min, max)`: whichever triangle asks second reuses the first
//! midpoint. Without that sharing the rebuilt mesh cracks open along every
//! split edge.

use std::collections::HashMap;

use super::paint::PaintNode;

/// The subdivided mesh. `vertices` starts with the input vertices, so every
/// child face keeps the winding and the vertex order of its parent; `slots[i]`
/// is the extruder slot of `faces[i]`, and both vectors are in traversal order.
#[derive(Debug, Clone, PartialEq)]
pub struct Expanded {
    pub vertices: Vec<[f32; 3]>,
    pub faces: Vec<[u32; 3]>,
    pub slots: Vec<u8>,
}

/// Hard ceiling on tree nesting, matching [`super::paint`]'s decoder. Real
/// files peak at 9 levels; the bound keeps the recursion off the stack limit
/// for a hand-built tree. `expand` is infallible, so a tree past the cap is
/// dropped rather than reported — [`super::paint::decode`] already rejects one.
const MAX_DEPTH: u8 = 64;

/// Expand every base triangle into the faces its `paint_color` tree describes.
///
/// `trees[i]` belongs to `base_faces[i]`; `None` means that triangle carries no
/// `paint_color` and stays a single unpainted (slot 0) face. Faces produced by
/// one tree are adjacent in the output and ordered by child slot.
pub fn expand(
    base_vertices: &[[f32; 3]],
    base_faces: &[[u32; 3]],
    trees: &[Option<PaintNode>],
) -> Expanded {
    let mut builder = Builder {
        vertices: base_vertices.to_vec(),
        midpoints: HashMap::new(),
        faces: Vec::with_capacity(base_faces.len()),
        slots: Vec::with_capacity(base_faces.len()),
    };
    for (index, face) in base_faces.iter().enumerate() {
        // A triangle pointing outside the vertex array cannot be placed; it is
        // dropped rather than indexed, because `expand` has no error channel.
        if face.iter().any(|v| *v as usize >= base_vertices.len()) {
            continue;
        }
        match trees.get(index).and_then(Option::as_ref) {
            Some(tree) => builder.walk(*face, tree, 1),
            None => {
                builder.faces.push(*face);
                builder.slots.push(0);
            }
        }
    }
    Expanded {
        vertices: builder.vertices,
        faces: builder.faces,
        slots: builder.slots,
    }
}

/// The accumulating mesh plus the shared-midpoint cache.
struct Builder {
    vertices: Vec<[f32; 3]>,
    /// `(min, max)` base/derived vertex index pair → midpoint vertex index.
    midpoints: HashMap<(u32, u32), u32>,
    faces: Vec<[u32; 3]>,
    slots: Vec<u8>,
}

impl Builder {
    /// The midpoint of the edge `(a, b)`, created once and then shared.
    ///
    /// `a` and `b` are indices into [`Builder::vertices`], not into the base
    /// vertices: a nested split takes the midpoint of an edge that already ends
    /// in a midpoint created by its parent.
    fn midpoint(&mut self, a: u32, b: u32) -> u32 {
        let key = if a < b { (a, b) } else { (b, a) };
        if let Some(&index) = self.midpoints.get(&key) {
            return index;
        }
        let pa = self.vertices[a as usize];
        let pb = self.vertices[b as usize];
        let index = self.vertices.len() as u32;
        self.vertices.push([
            (pa[0] + pb[0]) * 0.5,
            (pa[1] + pb[1]) * 0.5,
            (pa[2] + pb[2]) * 0.5,
        ]);
        self.midpoints.insert(key, index);
        index
    }

    /// Port of `TriangleSelector::perform_split`.
    ///
    /// `tri` is the triangle as it currently stands (base vertices, or base
    /// vertices and midpoints from an ancestor split); side `i` is the edge
    /// opposite `tri[i]`. The `insert` positions below are upstream's, and the
    /// child triples are transcribed verbatim from it.
    fn walk(&mut self, tri: [u32; 3], node: &PaintNode, depth: u8) {
        if depth > MAX_DEPTH {
            return;
        }
        if node.split == 0 {
            self.faces.push(tri);
            self.slots.push(node.state);
            return;
        }
        // decode() guarantees a special side below 3 on a 1- or 2-side split
        // and 0 on a 3-side split; anything else would index the sides out of
        // range, so it is dropped instead.
        let side = usize::from(node.special_side);
        if side >= 3 {
            return;
        }

        // Re-index so the special side is the edge opposite `v[0]`.
        let mut v = vec![tri[side], tri[(side + 1) % 3], tri[(side + 2) % 3]];
        match node.split {
            // Split the edge opposite `v[0]` (the "special" side): two children.
            1 => {
                let m = self.midpoint(v[2], v[1]);
                v.insert(2, m);
                self.child([v[0], v[1], v[2]], node, 0, depth);
                self.child([v[2], v[3], v[0]], node, 1, depth);
            }
            // Two split sides; `v[0]` is the vertex the kept side is opposite.
            2 => {
                let m1 = self.midpoint(v[1], v[0]);
                v.insert(1, m1);
                let m2 = self.midpoint(v[0], v[3]);
                v.insert(4, m2);
                self.child([v[0], v[1], v[4]], node, 0, depth);
                self.child([v[1], v[2], v[4]], node, 1, depth);
                self.child([v[2], v[3], v[4]], node, 2, depth);
            }
            // All three sides: three corners plus the middle triangle (child 3),
            // whose vertices are the three midpoints.
            3 => {
                let m0 = self.midpoint(v[1], v[0]);
                v.insert(1, m0);
                let m1 = self.midpoint(v[3], v[2]);
                v.insert(3, m1);
                let m2 = self.midpoint(v[0], v[4]);
                v.insert(5, m2);
                self.child([v[0], v[1], v[5]], node, 0, depth);
                self.child([v[1], v[2], v[3]], node, 1, depth);
                self.child([v[3], v[4], v[5]], node, 2, depth);
                self.child([v[1], v[3], v[5]], node, 3, depth);
            }
            // Only 1..=3 exist; decode() rejects anything else.
            _ => {}
        }
    }

    /// Recurse into child `slot`, if the tree carries it (a node whose children
    /// disagree with its `split` cannot come out of [`super::paint::decode`]).
    fn child(&mut self, tri: [u32; 3], node: &PaintNode, slot: usize, depth: u8) {
        if let Some(child) = node.children.get(slot) {
            self.walk(tri, child, depth + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::expand;
    use crate::threemf::paint::{decode, PaintNode};

    fn area(v: &[[f32; 3]], f: [u32; 3]) -> f32 {
        let a = v[f[0] as usize];
        let b = v[f[1] as usize];
        let c = v[f[2] as usize];
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() * 0.5
    }

    fn leaf(state: u8) -> PaintNode {
        PaintNode { split: 0, special_side: 0, state, children: vec![] }
    }

    fn split(special_side: u8, children: Vec<PaintNode>) -> PaintNode {
        PaintNode { split: (children.len() - 1) as u8, special_side, state: 0, children }
    }

    #[test]
    fn an_unpainted_triangle_stays_one_face_with_slot_zero() {
        let v = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let out = expand(&v, &[[0, 1, 2]], &[None]);
        assert_eq!(out.faces, vec![[0, 1, 2]]);
        assert_eq!(out.slots, vec![0]);
    }

    #[test]
    fn a_leaf_tree_keeps_its_slot() {
        let v = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let out = expand(&v, &[[0, 1, 2]], &[Some(leaf(4))]);
        assert_eq!(out.faces.len(), 1);
        assert_eq!(out.slots, vec![4]);
    }

    #[test]
    fn one_split_side_adds_one_midpoint_and_two_children() {
        // special_side = the split side (side i is the edge OPPOSITE vertex i).
        let v = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let out = expand(&v, &[[0, 1, 2]], &[Some(split(0, vec![leaf(1), leaf(2)]))]);
        assert_eq!(out.vertices.len(), 4);
        assert_eq!(out.faces.len(), 2);
        // Side 0 is edge (1,2), so vertex 3 is its midpoint. Child 0 keeps the
        // parent's winding around vertex 0, child 1 around the split edge.
        assert_eq!(out.faces, vec![[0, 1, 3], [3, 2, 0]]);
        assert_eq!(out.slots, vec![1, 2]);
        let total: f32 = out.faces.iter().map(|f| area(&out.vertices, *f)).sum();
        assert!((total - 0.5).abs() < 1e-6, "children must tile the parent, got {total}");
        assert_eq!(out.slots.len(), 2);
    }

    #[test]
    fn two_split_sides_keep_the_special_side_edge_unsplit() {
        // special_side = the KEPT side for a 2-side split (TriangleSelector::set_division).
        let v = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let out = expand(&v, &[[0, 1, 2]], &[Some(split(2, vec![leaf(1), leaf(2), leaf(3)]))]);
        assert_eq!(out.vertices.len(), 5, "3 base + 2 midpoints; the kept side has none");
        assert_eq!(out.faces.len(), 3);
        // Kept side 2 is edge (0,1); the midpoints are on (2,0) and (2,1), i.e.
        // vertices 3 and 4. Child 0 is the corner at vertex 2, children 1 and 2
        // split the quad left around the unsplit edge (0,1).
        assert_eq!(out.faces, vec![[2, 3, 4], [3, 0, 4], [0, 1, 4]]);
        assert_eq!(out.slots, vec![1, 2, 3]);
        let total: f32 = out.faces.iter().map(|f| area(&out.vertices, *f)).sum();
        assert!((total - 0.5).abs() < 1e-6, "children must tile the parent, got {total}");
    }

    #[test]
    fn three_split_sides_produce_three_corners_and_a_middle_child() {
        let v = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let out = expand(&v, &[[0, 1, 2]], &[Some(split(0, vec![leaf(1), leaf(2), leaf(3), leaf(4)]))]);
        assert_eq!(out.vertices.len(), 6);
        assert_eq!(out.faces.len(), 4);
        // Midpoints: 3 on (0,1), 4 on (1,2), 5 on (0,2). Children 0..2 are the
        // corners around vertices 0, 1, 2; child 3 is the middle triangle.
        assert_eq!(out.faces, vec![[0, 3, 5], [3, 1, 4], [4, 2, 5], [3, 4, 5]]);
        // child slot 3 is the middle triangle: all three of its vertices are new.
        let middle = out.faces[3];
        assert!(middle.iter().all(|i| *i >= 3), "middle child must use the three midpoints: {middle:?}");
        assert_eq!(out.slots, vec![1, 2, 3, 4]);
        let total: f32 = out.faces.iter().map(|f| area(&out.vertices, *f)).sum();
        assert!((total - 0.5).abs() < 1e-6, "children must tile the parent, got {total}");
    }

    #[test]
    fn a_shared_edge_produces_exactly_one_midpoint() {
        // Two triangles sharing edge (0,1): for each, that edge is opposite vertex 2.
        let v = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, -1.0, 0.0]];
        let faces = vec![[0u32, 1, 2], [1, 0, 3]];
        let t = Some(split(2, vec![leaf(1), leaf(2)]));
        let out = expand(&v, &faces, &[t.clone(), t]);
        assert_eq!(out.vertices.len(), 5, "4 base + EXACTLY one shared midpoint (no crack)");
    }

    #[test]
    fn recursion_stays_shallow_and_correct_on_a_real_value() {
        let v = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let tree = decode("000800303").expect("real value");
        let out = expand(&v, &[[0, 1, 2]], &[Some(tree)]);
        assert_eq!(out.faces.len(), 7);
        let total: f32 = out.faces.iter().map(|f| area(&out.vertices, *f)).sum();
        assert!((total - 0.5).abs() < 1e-6);
    }
}

