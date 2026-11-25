import { BufferAttribute, BufferGeometry } from "three";
import { MeshBVH, type MeshBVHOptions } from "three-mesh-bvh";

self.onmessage = (event: MessageEvent<{ positions: Float32Array; indices: Uint32Array }>) => {
  try {
    const { positions, indices } = event.data;
    const geometry = new BufferGeometry();
    geometry.setAttribute("position", new BufferAttribute(positions, 3));
    geometry.setIndex(new BufferAttribute(indices, 1));
    // Indirection preserves the face IDs used by the coloring backend.
    const options: MeshBVHOptions & { indirect: boolean } = { indirect: true };
    const tree = new MeshBVH(geometry, options);
    const { index: _, ...bounds } = MeshBVH.serialize(tree, { cloneBuffers: false });
    const indirect = (bounds as typeof bounds & { indirectBuffer: Uint32Array | Uint16Array }).indirectBuffer;
    self.postMessage({ bounds }, { transfer: [...bounds.roots, indirect.buffer as ArrayBuffer] });
  } catch (error) {
    self.postMessage({ error: String(error) });
  }
};
