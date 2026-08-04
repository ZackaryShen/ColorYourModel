import { useCallback, useMemo, useRef } from "react";
import * as THREE from "three";
import { useAppStore } from "../store/appStore";
import { log } from "../utils/logger";

// Segment color palette (matches SegmentsPanel SEGMENT_COLORS, as hex RGB)
const SEGMENT_COLORS_RGB: [number, number, number][] = [
  [231, 76, 60], [52, 152, 219], [46, 204, 113], [241, 196, 15], [155, 89, 182],
  [230, 126, 34], [26, 188, 156], [233, 30, 99], [0, 188, 212], [139, 195, 74],
  [255, 152, 0], [121, 85, 72], [96, 125, 139], [255, 87, 34], [103, 58, 183],
];

/**
 * Hook for managing mesh geometry and Three.js BufferGeometry
 */
export function useMesh() {
  const meshData = useAppStore((s) => s.meshData);
  const segmentView = useAppStore((s) => s.segmentView);
  const selectedSegment = useAppStore((s) => s.selectedSegment);
  const geometryRef = useRef<THREE.BufferGeometry | null>(null);
  // Cache of the non-indexed geometry derived from `baseGeometry`. Built ONCE
  // per base geometry (mesh load); subsequent view/selection toggles only copy
  // the color array — preserving the iteration-9 lag fix while fixing the
  // indexed/per-face-color mismatch (iteration 14, paint-elsewhere bug).
  const nonIndexedRef = useRef<THREE.BufferGeometry | null>(null);
  // Live handle on the memoized paint-view color buffer. `updateFaceColors`
  // patches it incrementally so switching to the segment view and back does not
  // repaint from a STALE buffer (the memo only recomputes when `meshData`
  // changes, which an in-place paint deliberately avoids). O(k) per stroke.
  const paintColorRef = useRef<Float32Array | null>(null);

  // Base geometry: position + index + normals. Depends ONLY on meshData, so
  // toggling the segment/paint view or selecting a region never reallocates
  // buffers or recomputes normals (the previous source of view-switch lag).
  const baseGeometry = useMemo(() => {
    if (!meshData) {
      log.debug("useMesh", "buildBaseGeometry: no meshData");
      return null;
    }
    log.info("useMesh", "buildBaseGeometry", {
      vertices: meshData.vertices.length / 3,
      faces: meshData.faces.length / 3,
    });
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute(
      "position",
      new THREE.Float32BufferAttribute(new Float32Array(meshData.vertices), 3)
    );
    geometry.setIndex(new THREE.BufferAttribute(new Uint32Array(meshData.faces), 1));
    geometry.computeVertexNormals();
    geometry.computeBoundingSphere();
    geometryRef.current = geometry;
    return geometry;
    // Deps are the STABLE sub-references (vertices/faces/bbox), NOT the whole
    // `meshData` object. `updateSegmentLabels` spreads `meshData` (keeping these
    // array refs) and only swaps `segmentLabels`/`faceColors`, so a label update
    // no longer triggers a full geometry rebuild + normal recompute — the
    // dominant cause of the "添加分区出现得较晚" lag (iteration 9, problem 2).
  }, [meshData?.vertices, meshData?.faces, meshData?.bbox]);

  // Precompute the two full color buffers ONCE per (meshData / labels /
  // selection). Toggling the segment/paint view then only performs a native
  // typed-array copy (no per-face JS loop), eliminating the view-switch lag
  // that came from recomputing colors on every toggle.
  const paintColorArray = useMemo(() => {
    if (!meshData) return null;
    const fc = meshData.faceColors;
    const faceCount = meshData.faceCount;
    const arr = new Float32Array(faceCount * 9);
    for (let i = 0; i < faceCount; i++) {
      const r = fc[i * 4] / 255;
      const g = fc[i * 4 + 1] / 255;
      const b = fc[i * 4 + 2] / 255;
      for (let v = 0; v < 3; v++) {
        arr[i * 9 + v * 3] = r;
        arr[i * 9 + v * 3 + 1] = g;
        arr[i * 9 + v * 3 + 2] = b;
      }
    }
    paintColorRef.current = arr;
    return arr;
  }, [meshData?.faceColors, meshData?.faceCount]);

  const segmentColorArray = useMemo(() => {
    // Gate: paint view never reads this buffer (iteration 23, REFUTE B12).
    // Skipping the 54MB Float32Array + 1.5M-face loop when unnecessary
    // eliminates the dominant allocation spike on region finalize / undo.
    if (!meshData || !segmentView) return null;
    const faceCount = meshData.faceCount;
    const labels = meshData.segmentLabels;
    const hasLabels = labels && labels.length === faceCount;
    const arr = new Float32Array(faceCount * 9);
    for (let i = 0; i < faceCount; i++) {
      if (hasLabels) {
        const segId = labels[i];
        const ci = segId % SEGMENT_COLORS_RGB.length;
        let [r, g, b] = SEGMENT_COLORS_RGB[ci];
        // Highlight selected segment (brighten); dim the rest for contrast.
        if (selectedSegment !== null && segId === selectedSegment) {
          r = Math.min(255, r + 60);
          g = Math.min(255, g + 60);
          b = Math.min(255, b + 60);
        } else if (selectedSegment !== null) {
          r = Math.floor(r * 0.4);
          g = Math.floor(g * 0.4);
          b = Math.floor(b * 0.4);
        }
        for (let v = 0; v < 3; v++) {
          arr[i * 9 + v * 3] = r / 255;
          arr[i * 9 + v * 3 + 1] = g / 255;
          arr[i * 9 + v * 3 + 2] = b / 255;
        }
      } else {
        // No segmentation yet: keep the paint colors so the view is not blank.
        const r = meshData.faceColors[i * 4] / 255;
        const g = meshData.faceColors[i * 4 + 1] / 255;
        const b = meshData.faceColors[i * 4 + 2] / 255;
        for (let v = 0; v < 3; v++) {
          arr[i * 9 + v * 3] = r;
          arr[i * 9 + v * 3 + 1] = g;
          arr[i * 9 + v * 3 + 2] = b;
        }
      }
    }
    return arr;
  }, [meshData, selectedSegment, segmentView]);

  // Per-face colors. The rendered geometry is NON-INDEXED: each face owns its
  // own 3 consecutive vertices, so the per-vertex color attribute laid out as
  // `[f0v0,f0v1,f0v2, f1v0,...]` (which `updateFaceColors` and the precomputed
  // color arrays use) maps 1:1 to `faceIdx*3+v`. With an INDEXED geometry the
  // position buffer holds shared vertices, so `faceIdx*3+v` landed on unrelated
  // shared vertices and painted entirely the wrong faces ("ring at hand, red at
  // the arm", iteration 14). `baseGeometry.toNonIndexed()` expands the indexed
  // buffer into this flat layout while keeping the SAME triangle/face order, so
  // the raycaster's `faceIndex` still equals the backend face index.
  const buildGeometry = useCallback((): THREE.BufferGeometry | null => {
    if (!baseGeometry || !meshData) {
      log.debug("useMesh", "buildGeometry: no base geometry");
      return null;
    }
    const faceCount = meshData.faceCount;

    // Rebuild the non-indexed shell only when the underlying base geometry
    // changes (mesh (re)load). Toggling segment/paint view or selection only
    // refreshes the color array below — cheap, preserves the lag fix.
    if (!nonIndexedRef.current || nonIndexedRef.current.userData.baseId !== baseGeometry.uuid) {
      if (nonIndexedRef.current) nonIndexedRef.current.dispose();
      const g = baseGeometry.toNonIndexed();
      g.computeBoundingSphere();
      g.userData.baseId = baseGeometry.uuid;
      nonIndexedRef.current = g;
    }
    const geometry = nonIndexedRef.current;

    let colorAttr = geometry.getAttribute("color") as THREE.Float32BufferAttribute | undefined;
    if (!colorAttr || colorAttr.count !== faceCount * 3) {
      colorAttr = new THREE.Float32BufferAttribute(new Float32Array(faceCount * 9), 3);
      geometry.setAttribute("color", colorAttr);
    }

    const src = segmentView && segmentColorArray ? segmentColorArray : paintColorArray;
    if (src) {
      (colorAttr.array as Float32Array).set(src);
    }
    colorAttr.needsUpdate = true;
    geometryRef.current = geometry;
    return geometry;
  }, [baseGeometry, meshData, segmentView, segmentColorArray, paintColorArray]);

  const updateFaceColors = useCallback(
    (updatedFaces: number[], updatedColors: number[][]) => {
      if (!geometryRef.current) {
        log.warn("useMesh", "updateFaceColors: no geometry ref");
        return;
      }

      const colorAttr = geometryRef.current.getAttribute("color") as THREE.Float32BufferAttribute;
      if (!colorAttr) return;

      log.debug("useMesh", "updateFaceColors", { count: updatedFaces.length });

      // Track the touched vertex range so we upload ONLY that span, not the whole
      // color buffer. The color buffer is `faceCount * 9` floats; on a 200k-face
      // mesh a full re-upload is 7.2 MB every frame — the dominant steady-state GPU
      // cost (iteration 16, REFUTE major-3). The renderer auto-clears updateRanges
      // after each upload, so we only add the current batch here (no manual clear).
      let vMin = Infinity;
      let vMax = 0;
      const cache = paintColorRef.current;

      for (let j = 0; j < updatedFaces.length; j++) {
        const faceIdx = updatedFaces[j];
        const c = updatedColors[j];
        const r = c[0] / 255;
        const g = c[1] / 255;
        const b = c[2] / 255;

        for (let v = 0; v < 3; v++) {
          const vi = faceIdx * 3 + v;
          colorAttr.setXYZ(vi, r, g, b);
          if (cache) {
            const o = faceIdx * 9 + v * 3;
            cache[o] = r;
            cache[o + 1] = g;
            cache[o + 2] = b;
          }
          if (vi < vMin) vMin = vi;
          if (vi > vMax) vMax = vi;
        }
      }

      // Write back to the store's canonical faceColors so undo snapshots,
      // redo and `finalizeSegment` all see the CURRENT paint state. In-place
      // mutation keeps the `meshData` reference stable (no camera reset, no
      // geometry rebuild) — iteration 18, B2/B3.
      useAppStore.getState().applyFaceColors(updatedFaces, updatedColors);

      if (vMin !== Infinity) {
        // addUpdateRange start/count are in ARRAY ELEMENTS (floats); vertex vi →
        // offset vi*3, itemSize 3.
        colorAttr.addUpdateRange(vMin * 3, (vMax - vMin + 1) * 3);
      }
      colorAttr.needsUpdate = true;
    },
    []
  );

  return { buildGeometry, updateFaceColors, geometryRef };
}
