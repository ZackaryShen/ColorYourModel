import { useCallback, useRef } from "react";
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

  const buildGeometry = useCallback((): THREE.BufferGeometry | null => {
    if (!meshData) {
      log.debug("useMesh", "buildGeometry: no meshData");
      return null;
    }

    log.info("useMesh", "buildGeometry start", {
      vertices: meshData.vertices.length / 3,
      faces: meshData.faces.length / 3,
      faceColors: meshData.faceColors.length / 4,
      segmentView,
      hasLabels: meshData.segmentLabels.length > 0,
    });

    const geometry = new THREE.BufferGeometry();

    // Vertices are flat [x0,y0,z0, x1,y1,z1, ...]
    const posArray = new Float32Array(meshData.vertices);
    geometry.setAttribute("position", new THREE.Float32BufferAttribute(posArray, 3));

    // Face indices are flat [f0v0,f0v1,f0v2, ...]
    const indexArray = new Uint32Array(meshData.faces);
    geometry.setIndex(new THREE.BufferAttribute(indexArray, 1));

    const faceCount = meshData.faceCount;
    const colorArray = new Float32Array(faceCount * 3 * 3); // 3 verts * 3 channels
    const fc = meshData.faceColors;
    const labels = meshData.segmentLabels;
    const hasLabels = labels && labels.length === faceCount;

    // Choose color source: segment colors or face colors
    if (segmentView && hasLabels) {
      // Segment visualization mode
      for (let i = 0; i < faceCount; i++) {
        const segId = labels[i];
        const ci = segId % SEGMENT_COLORS_RGB.length;
        let [r, g, b] = SEGMENT_COLORS_RGB[ci];

        // Highlight selected segment (brighten by 30%)
        if (selectedSegment !== null && segId === selectedSegment) {
          r = Math.min(255, r + 60);
          g = Math.min(255, g + 60);
          b = Math.min(255, b + 60);
        } else if (selectedSegment !== null) {
          // Dim non-selected segments
          r = Math.floor(r * 0.4);
          g = Math.floor(g * 0.4);
          b = Math.floor(b * 0.4);
        }

        const rf = r / 255, gf = g / 255, bf = b / 255;
        for (let v = 0; v < 3; v++) {
          colorArray[i * 9 + v * 3 + 0] = rf;
          colorArray[i * 9 + v * 3 + 1] = gf;
          colorArray[i * 9 + v * 3 + 2] = bf;
        }
      }
      log.info("useMesh", "Segment colors applied");
    } else {
      // Normal face color mode
      for (let i = 0; i < faceCount; i++) {
        const r = fc[i * 4] / 255;
        const g = fc[i * 4 + 1] / 255;
        const b = fc[i * 4 + 2] / 255;

        for (let v = 0; v < 3; v++) {
          colorArray[i * 9 + v * 3 + 0] = r;
          colorArray[i * 9 + v * 3 + 1] = g;
          colorArray[i * 9 + v * 3 + 2] = b;
        }
      }
    }
    geometry.setAttribute("color", new THREE.Float32BufferAttribute(colorArray, 3));

    geometry.computeVertexNormals();
    geometry.computeBoundingSphere();

    log.info("useMesh", "buildGeometry done", {
      indexCount: geometry.index ? geometry.index.count : 0,
    });

    geometryRef.current = geometry;
    return geometry;
  }, [meshData, segmentView, selectedSegment]);

  const updateFaceColors = useCallback(
    (updatedFaces: number[], updatedColors: number[][]) => {
      if (!geometryRef.current) {
        log.warn("useMesh", "updateFaceColors: no geometry ref");
        return;
      }

      log.debug("useMesh", "updateFaceColors", { count: updatedFaces.length });

      const colorAttr = geometryRef.current.getAttribute("color") as THREE.Float32BufferAttribute;
      if (!colorAttr) return;

      for (let j = 0; j < updatedFaces.length; j++) {
        const faceIdx = updatedFaces[j];
        const c = updatedColors[j];
        const r = c[0] / 255;
        const g = c[1] / 255;
        const b = c[2] / 255;

        for (let v = 0; v < 3; v++) {
          colorAttr.setXYZ(faceIdx * 3 + v, r, g, b);
        }
      }

      colorAttr.needsUpdate = true;
    },
    []
  );

  return { buildGeometry, updateFaceColors, geometryRef };
}
