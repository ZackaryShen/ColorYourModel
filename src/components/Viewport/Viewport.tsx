import { Canvas, useThree } from "@react-three/fiber";
import { OrbitControls } from "@react-three/drei";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as THREE from "three";
import { useAppStore } from "../../store/appStore";
import { useMesh } from "../../hooks/useMesh";
import { usePaintTool } from "../../hooks/usePaintTool";
import { useTauriCommand } from "../../hooks/useTauriCommand";
import { log } from "../../utils/logger";
import { useT } from "../../i18n";

// Shared controls reference — set by ControlsBridge, read by MeshDisplay
let _orbitControls: (THREE.EventDispatcher & {
  enableRotate: boolean;
  enablePan: boolean;
  mouseButtons: { LEFT: number; MIDDLE: number; RIGHT: number };
}) | null = null;

// Is Space key currently held? (set by MeshDisplay keyboard handler)
let _spaceHeld = false;

// ─── GPU Picker Hook ──────────────────────────────────────────────
function useGpuPicker(
  meshRef: React.RefObject<THREE.Mesh | null>,
  onFacePicked: (faceId: number) => void
) {
  const { gl, camera, size } = useThree();
  const pickTargetRef = useRef<THREE.WebGLRenderTarget | null>(null);
  // Persistent pick geometry (avoids 18MB clone per click)
  const pickGeoRef = useRef<THREE.BufferGeometry | null>(null);
  const pickMatRef = useRef<THREE.MeshBasicMaterial | null>(null);

  useEffect(() => {
    const dpr = gl.getPixelRatio();
    const w = Math.floor(size.width * dpr);
    const h = Math.floor(size.height * dpr);

    if (pickTargetRef.current) pickTargetRef.current.dispose();
    pickTargetRef.current = new THREE.WebGLRenderTarget(w, h, {
      format: THREE.RGBAFormat,
      type: THREE.UnsignedByteType,
    });

    return () => {
      pickTargetRef.current?.dispose();
      pickTargetRef.current = null;
      pickGeoRef.current?.dispose();
      pickGeoRef.current = null;
      pickMatRef.current?.dispose();
      pickMatRef.current = null;
    };
  }, [size.width, size.height, gl]);

  const pick = useCallback(
    (clientX: number, clientY: number) => {
      try {
        const target = pickTargetRef.current;
        const mesh = meshRef.current;
        if (!target || !mesh) {
          log.warn("GpuPicker", "pick skipped: no target or mesh",
            { hasTarget: !!target, hasMesh: !!mesh });
          return;
        }

        const geo = mesh.geometry;
        const index = geo.index;
        if (!index) {
          log.warn("GpuPicker", "pick skipped: no index buffer");
          return;
        }

        mesh.updateWorldMatrix(true, false);

        // Reuse or create pick geometry (share position/index buffers — avoids clone)
        if (!pickGeoRef.current) {
          pickGeoRef.current = new THREE.BufferGeometry();
        }
        const pickGeo = pickGeoRef.current;
        const sourcePos = geo.getAttribute("position");
        pickGeo.setAttribute("position", new THREE.Float32BufferAttribute(sourcePos.array, 3));
        pickGeo.setIndex(new THREE.BufferAttribute(geo.index!.array, 1));

        const faceCount = index.count / 3;
        const idColors = new Float32Array(faceCount * 3 * 3);
        for (let i = 0; i < faceCount; i++) {
          const id = i + 1;
          const r = (id & 0xff) / 255;
          const g = ((id >> 8) & 0xff) / 255;
          const b = ((id >> 16) & 0xff) / 255;
          for (let v = 0; v < 3; v++) {
            idColors[i * 9 + v * 3 + 0] = r;
            idColors[i * 9 + v * 3 + 1] = g;
            idColors[i * 9 + v * 3 + 2] = b;
          }
        }
        pickGeo.setAttribute("color", new THREE.Float32BufferAttribute(idColors, 3));

        const pickMat = pickMatRef.current || (pickMatRef.current = new THREE.MeshBasicMaterial({ vertexColors: true }));
        const pickMesh = new THREE.Mesh(pickGeo, pickMat);
        pickMesh.matrix.copy(mesh.matrixWorld);
        pickMesh.matrixAutoUpdate = false;

        const pickScene = new THREE.Scene();
        pickScene.add(pickMesh);

        const dpr = gl.getPixelRatio();
        const rtW = target.width;
        const rtH = target.height;
        gl.setRenderTarget(target);
        gl.setViewport(0, 0, rtW, rtH);
        gl.clear();
        gl.render(pickScene, camera);

        const rect = gl.domElement.getBoundingClientRect();
        let glX = Math.floor((clientX - rect.left) * dpr);
        let glY = Math.floor((rect.height - (clientY - rect.top)) * dpr);
        // Clamp to valid pixel range
        glX = Math.max(0, Math.min(glX, rtW - 1));
        glY = Math.max(0, Math.min(glY, rtH - 1));

        const pixel = new Uint8Array(4);
        gl.readRenderTargetPixels(target, glX, glY, 1, 1, pixel);
        // Restore default framebuffer + full viewport
        gl.setRenderTarget(null);
        gl.setViewport(0, 0, target.width, target.height);

        const faceId = pixel[0] | (pixel[1] << 8) | (pixel[2] << 16);
        log.info("GpuPicker", "pick result",
          { clientX, clientY, glX, glY, pixel: [pixel[0], pixel[1], pixel[2], pixel[3]], faceId, faceCount });

        if (faceId > 0 && faceId <= faceCount) {
          onFacePicked(faceId - 1);
        }

        // Don't dispose pickGeo/pickMat — reused across picks
      } catch (err) {
        log.error("GpuPicker", "pick threw error", { error: String(err) });
      }
    },
    [gl, camera, meshRef, onFacePicked]
  );

  return pick;
}

// ─── Camera Auto-Fit ──────────────────────────────────────────────
function CameraFit() {
  const { camera } = useThree();
  const meshData = useAppStore((s) => s.meshData);

  useEffect(() => {
    if (!meshData?.bbox) return;
    const { min, max } = meshData.bbox;
    // STL bbox is Z-up. After group rotation -PI/2 around X:
    // Three.js Y = STL Z, Three.js Z = -STL Y
    const cx = (min[0] + max[0]) / 2;
    const cy = (min[2] + max[2]) / 2;
    const cz = -(min[1] + max[1]) / 2;
    const dimX = max[0] - min[0];
    const dimY = max[2] - min[2];
    const dimZ = max[1] - min[1];
    const maxDim = Math.max(dimX, dimY, dimZ);
    if (maxDim < 0.001) return;
    const fov = (camera as THREE.PerspectiveCamera).fov * (Math.PI / 180);
    const dist = (maxDim / (2 * Math.tan(fov / 2))) * 1.5;

    camera.position.set(cx + dist * 0.5, cy + dist * 0.5, cz + dist);
    camera.lookAt(cx, cy, cz);
    camera.updateProjectionMatrix();

    log.info("CameraFit", "Camera positioned", { center: [cx, cy, cz], maxDim, dist: +dist.toFixed(1) });
  }, [meshData, camera]);

  return null;
}

// ─── Controls Bridge ──────────────────────────────────────────────
function ControlsBridge() {
  const ref = useRef<any>(null);

  useEffect(() => {
    _orbitControls = ref.current;
    return () => { _orbitControls = null; };
  }, []);

  return <OrbitControls ref={ref} makeDefault enableRotate enablePan enableZoom />;
}

// ─── Adaptive Grid ────────────────────────────────────────────────
function AdaptiveGrid() {
  const meshData = useAppStore((s) => s.meshData);

  if (!meshData?.bbox) {
    return <gridHelper args={[200, 20, 0x555555, 0x333333]} />;
  }

  const { min, max } = meshData.bbox;
  // STL X → Three.js X (index 0), STL Y → Three.js -Z (index 1)
  const spanX = max[0] - min[0];
  const spanZ = max[1] - min[1]; // STL Y = ground depth
  const gridSize = Math.ceil(Math.max(spanX, spanZ) * 2.5 / 10) * 10; // round up to 10
  const centerX = (min[0] + max[0]) / 2;
  const centerZ = -(min[1] + max[1]) / 2; // STL Y → -Three.js Z
  const divisions = Math.max(10, Math.min(40, Math.ceil(gridSize / 10)));

  return (
    <group position={[centerX, 0, centerZ]}>
      <gridHelper args={[gridSize, divisions, 0x555555, 0x333333]} />
    </group>
  );
}

// ─── Brush Cursor (hover radius preview) ──────────────────────────
interface HoverInfo {
  position: THREE.Vector3;
  normal: THREE.Vector3;
}

function BrushCursor({ hoverInfo, brushRadius, color }: {
  hoverInfo: HoverInfo | null;
  brushRadius: number;
  color: [number, number, number, number];
}) {
  if (!hoverInfo) return null;

  // Create ring geometry and orient it to face normal
  const quaternion = useMemo(() => {
    const q = new THREE.Quaternion();
    q.setFromUnitVectors(new THREE.Vector3(0, 0, 1), hoverInfo.normal);
    return q;
  }, [hoverInfo.normal]);

  const ringColor = new THREE.Color(color[0] / 255, color[1] / 255, color[2] / 255);

  return (
    <group position={hoverInfo.position} quaternion={quaternion}>
      <ringGeometry args={[brushRadius * 0.9, brushRadius, 32]} />
      <meshBasicMaterial
        color={ringColor}
        transparent
        opacity={0.5}
        side={THREE.DoubleSide}
        depthTest={false}
      />
    </group>
  );
}

// ─── Segment Outline (boundary edges for selected segment) ────────
function SegmentOutline({ meshData, selectedSegment }: {
  meshData: { faces: number[]; segmentLabels: number[]; vertices: number[] };
  selectedSegment: number;
}) {
  const geometry = useMemo(() => {
    const faces = meshData.faces;
    const labels = meshData.segmentLabels;
    const verts = meshData.vertices;
    const faceCount = faces.length / 3;

    // Build edge → face map: edge key "minV_maxV" → list of face indices
    const edgeToFace = new Map<string, number[]>();
    for (let f = 0; f < faceCount; f++) {
      const v0 = faces[f * 3], v1 = faces[f * 3 + 1], v2 = faces[f * 3 + 2];
      const edges = [
        [Math.min(v0, v1), Math.max(v0, v1)],
        [Math.min(v1, v2), Math.max(v1, v2)],
        [Math.min(v2, v0), Math.max(v2, v0)],
      ];
      for (const [a, b] of edges) {
        const key = `${a}_${b}`;
        const list = edgeToFace.get(key);
        if (list) list.push(f);
        else edgeToFace.set(key, [f]);
      }
    }

    // Find boundary edges: edges where one face is in selectedSegment and the other isn't
    const boundaryVerts: number[] = [];
    for (const [key, faceList] of edgeToFace) {
      const [aStr, bStr] = key.split("_");
      const va = parseInt(aStr), vb = parseInt(bStr);

      const inSeg = faceList.some((f) => labels[f] === selectedSegment);
      const outSeg = faceList.some((f) => labels[f] !== selectedSegment);

      if (inSeg && (outSeg || faceList.length === 1)) {
        // This edge is on the boundary of the selected segment
        boundaryVerts.push(
          verts[va * 3], verts[va * 3 + 1], verts[va * 3 + 2],
          verts[vb * 3], verts[vb * 3 + 1], verts[vb * 3 + 2]
        );
      }
    }

    if (boundaryVerts.length === 0) return null;

    const geo = new THREE.BufferGeometry();
    geo.setAttribute("position", new THREE.Float32BufferAttribute(boundaryVerts, 3));
    return geo;
  }, [meshData.faces, meshData.segmentLabels, meshData.vertices, selectedSegment]);

  if (!geometry) return null;

  return (
    <lineSegments geometry={geometry}>
      <lineBasicMaterial color={0xffff00} linewidth={2} depthTest={false} />
    </lineSegments>
  );
}

// ─── Nearest local vertex (snap preview, frontend-only) ────────
/// Snap a model-local point to the nearest mesh vertex using the loaded
/// vertex array. Equivalent to the backend `nearest_vertex` (Euclidean kd-tree
/// lookup) but computed locally for zero-latency hover preview. Returns the
/// vertex position in LOCAL coords (same space as `meshData.vertices`).
function nearestVertexLocal(vertices: number[], p: THREE.Vector3): THREE.Vector3 | null {
  if (!vertices || vertices.length < 3) return null;
  let best = -1;
  let bestSq = Infinity;
  for (let i = 0; i < vertices.length; i += 3) {
    const dx = vertices[i] - p.x;
    const dy = vertices[i + 1] - p.y;
    const dz = vertices[i + 2] - p.z;
    const sq = dx * dx + dy * dy + dz * dz;
    if (sq < bestSq) {
      bestSq = sq;
      best = i / 3;
    }
  }
  if (best < 0) return null;
  return new THREE.Vector3(vertices[best * 3], vertices[best * 3 + 1], vertices[best * 3 + 2]);
}

// ─── Lasso Overlay (manual region selection) ────────────────────
function LassoOverlay({ points, preview, closing, dotSize, snap }: {
  points: THREE.Vector3[];
  preview: THREE.Vector3 | null;
  closing: boolean;
  dotSize: number;
  snap?: THREE.Vector3 | null;
}) {
  if (points.length === 0 && !preview) return null;

  // Polyline segments: consecutive clicked points + rubber band to cursor.
  const segPos: number[] = [];
  for (let i = 0; i + 1 < points.length; i++) {
    const a = points[i], b = points[i + 1];
    segPos.push(a.x, a.y, a.z, b.x, b.y, b.z);
  }
  if (preview && points.length > 0) {
    const last = points[points.length - 1];
    segPos.push(last.x, last.y, last.z, preview.x, preview.y, preview.z);
  }

  const dotPos: number[] = [];
  for (const p of points) dotPos.push(p.x, p.y, p.z);

  return (
    <group>
      {segPos.length > 0 && (
        <lineSegments>
          <bufferGeometry>
            <bufferAttribute attach="attributes-position" args={[new Float32Array(segPos), 3]} />
          </bufferGeometry>
          <lineBasicMaterial color={closing ? 0xffff00 : 0x4a9eff} linewidth={2} depthTest={false} />
        </lineSegments>
      )}
      {dotPos.length > 0 && (
        <>
          <points>
            <bufferGeometry>
              <bufferAttribute attach="attributes-position" args={[new Float32Array(dotPos), 3]} />
            </bufferGeometry>
            <pointsMaterial color={0x4a9eff} size={dotSize} sizeAttenuation depthTest={false} />
          </points>
          {/* Start point highlight — turns yellow when cursor is near to close */}
          <mesh position={points[0]}>
            <sphereGeometry args={[dotSize * 0.9, 12, 12]} />
            <meshBasicMaterial color={closing ? 0xffff00 : 0x00ff88} depthTest={false} />
          </mesh>
        </>
      )}
      {/* Snap preview: the nearest vertex where the next click will land */}
      {snap && (
        <mesh position={snap}>
          <sphereGeometry args={[dotSize * 1.15, 14, 14]} />
          <meshBasicMaterial color={0x00ffff} depthTest={false} />
        </mesh>
      )}
    </group>
  );
}

// ─── Main Mesh Display ────────────────────────────────────────────
function MeshDisplay() {
  const meshData = useAppStore((s) => s.meshData);
  const segmentView = useAppStore((s) => s.segmentView);
  const selectedSegment = useAppStore((s) => s.selectedSegment);
  const activeTool = useAppStore((s) => s.activeTool);
  const brushRadius = useAppStore((s) => s.brushRadius);
  const currentColor = useAppStore((s) => s.currentColor);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const { buildGeometry, updateFaceColors } = useMesh();
  const { paintFace } = usePaintTool();
  const { paintSegmentFace, finalizeSegment, manualRegionAddPoint, finalizeManualRegion, manualRegionUndo } = useTauriCommand();
  const t = useT();
  const meshRef = useRef<THREE.Mesh>(null);
  const isPainting = useRef(false);
  // Segment paint state: track current label + painted faces for dedup
  const currentSegLabelRef = useRef<number | null>(null);
  const segPaintedFacesRef = useRef<Set<number>>(new Set());
  const { gl, camera } = useThree();
  const raycaster = useMemo(() => new THREE.Raycaster(), []);
  const [hoverInfo, setHoverInfo] = useState<HoverInfo | null>(null);

  // Tools that show brush cursor
  const isBrushTool = activeTool === "brush" || activeTool === "spray" ||
    activeTool === "smart" || activeTool === "eraser";
  const isSegmentTool = activeTool === "segment";

  // Lasso (manual region) state
  const isLassoTool = activeTool === "lasso";
  const [lassoPoints, setLassoPoints] = useState<THREE.Vector3[]>([]);
  const [lassoPreview, setLassoPreview] = useState<THREE.Vector3 | null>(null);
  const [lassoClosing, setLassoClosing] = useState(false);
  const [lassoSnap, setLassoSnap] = useState<THREE.Vector3 | null>(null);
  const lassoSnapRef = useRef<THREE.Vector3 | null>(null);
  const lassoPointsRef = useRef<THREE.Vector3[]>([]);
  const lassoStartVertexRef = useRef<number | null>(null);

  const closeThreshold = useMemo(() => {
    const b = meshData?.bbox;
    if (!b) return 1.0;
    const dx = b.max[0] - b.min[0], dy = b.max[1] - b.min[1], dz = b.max[2] - b.min[2];
    return Math.max(0.01, Math.sqrt(dx * dx + dy * dy + dz * dz) * 0.012);
  }, [meshData?.bbox]);
  const dotSize = useMemo(() => {
    const b = meshData?.bbox;
    if (!b) return 1.0;
    const dx = b.max[0] - b.min[0], dy = b.max[1] - b.min[1], dz = b.max[2] - b.min[2];
    return Math.max(0.3, Math.sqrt(dx * dx + dy * dy + dz * dz) * 0.008);
  }, [meshData?.bbox]);

  // Set canvas cursor based on active tool
  useEffect(() => {
    const canvas = gl.domElement;
    if (isBrushTool && !segmentView) {
      canvas.style.cursor = "crosshair";
    } else if (isSegmentTool) {
      canvas.style.cursor = "cell";
    } else if (isLassoTool) {
      canvas.style.cursor = "crosshair";
    } else if (activeTool === "fill" || activeTool === "picker") {
      canvas.style.cursor = "pointer";
    } else {
      canvas.style.cursor = "default";
    }
    return () => { canvas.style.cursor = "default"; };
  }, [gl, isBrushTool, isSegmentTool, isLassoTool, segmentView, activeTool]);

  const geometry = useMemo(() => {
    log.info("MeshDisplay", "Building geometry", { segmentView });
    const geo = buildGeometry();
    if (geo) {
      const idx = geo.index;
      log.info("MeshDisplay", "Geometry ready", {
        faces: idx ? idx.count / 3 : 0,
        verts: geo.getAttribute("position")?.count ?? 0,
      });
    }
    return geo;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [buildGeometry, segmentView, selectedSegment, meshData?.segmentLabels]);

  // Raycast to surface, convert world hit → model-local coords.
  // Group is rotated -PI/2 about X, so worldToLocal yields local = (x, -z, y).
  const getLocalHit = useCallback(
    (clientX: number, clientY: number): THREE.Vector3 | null => {
      if (!meshRef.current || !geometry) return null;
      const rect = gl.domElement.getBoundingClientRect();
      const mouse = new THREE.Vector2(
        ((clientX - rect.left) / rect.width) * 2 - 1,
        -((clientY - rect.top) / rect.height) * 2 + 1
      );
      raycaster.setFromCamera(mouse, camera);
      const hits = raycaster.intersectObject(meshRef.current, false);
      if (hits.length === 0) return null;
      return meshRef.current.worldToLocal(hits[0].point.clone());
    },
    [gl, camera, raycaster, meshRef, geometry]
  );

  // Handle a lasso click: snap to vertex, append, or close the loop.
  const handleLassoClick = useCallback(
    async (local: THREE.Vector3) => {
      const res = await manualRegionAddPoint([local.x, local.y, local.z]);
      if (!res) return;
      const snapped = new THREE.Vector3(res.snapped[0], res.snapped[1], res.snapped[2]);
      const prev = lassoPointsRef.current;
      // Closure: clicked the start vertex again with >= 2 points already placed.
      if (prev.length >= 2 && res.vertexIndex === lassoStartVertexRef.current) {
        const pts = prev.map((p) => [p.x, p.y, p.z] as [number, number, number]);
        lassoPointsRef.current = [];
        setLassoPoints([]);
        lassoStartVertexRef.current = null;
        setLassoPreview(null);
        setLassoClosing(false);
        await finalizeManualRegion(pts);
        return;
      }
      if (prev.length === 0) lassoStartVertexRef.current = res.vertexIndex;
      const next = [...prev, snapped];
      lassoPointsRef.current = next;
      setLassoPoints(next);
      setStatusMessage(
        `套索：已选 ${next.length} 个点` + (next.length >= 2 ? "（点击起点闭合）" : "")
      );
    },
    [manualRegionAddPoint, finalizeManualRegion, setStatusMessage]
  );

  const handleFacePicked = useCallback(
    async (faceId: number) => {
      if (isSegmentTool) {
        // Segment paint brush: skip already-painted faces in this drag
        if (segPaintedFacesRef.current.has(faceId)) return;
        segPaintedFacesRef.current.add(faceId);

        const result = await paintSegmentFace(faceId, currentSegLabelRef.current ?? undefined);
        if (result) {
          // Track label for subsequent faces in this drag
          currentSegLabelRef.current = result.segmentLabel;
          // Incremental color update on GPU
          updateFaceColors([result.faceId], [result.color as unknown as [number, number, number, number]]);
        }
      } else {
        const result = await paintFace(faceId);
        if (result) {
          updateFaceColors(result.updatedFaces, result.updatedColors);
        }
      }
    },
    [paintFace, paintSegmentFace, updateFaceColors, isSegmentTool]
  );

  const pick = useGpuPicker(meshRef, handleFacePicked);

  // Space key → pan mode (FIX-2)
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.code === "Space" && !e.repeat) {
        e.preventDefault();
        _spaceHeld = true;
        if (_orbitControls) {
          _orbitControls.mouseButtons.LEFT = (THREE as any).MOUSE.PAN;
        }
      }
    };
    const onKeyUp = (e: KeyboardEvent) => {
      if (e.code === "Space") {
        _spaceHeld = false;
        if (_orbitControls) {
          _orbitControls.mouseButtons.LEFT = (THREE as any).MOUSE.LEFT;
        }
      }
    };
    const onBlur = () => {
      // Reset on window blur to prevent stuck state
      _spaceHeld = false;
      if (_orbitControls) {
        _orbitControls.mouseButtons.LEFT = (THREE as any).MOUSE.LEFT;
      }
    };
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
      window.removeEventListener("blur", onBlur);
    };
  }, []);

  // Pointer events for painting + hover
  useEffect(() => {
    const canvas = gl.domElement;

    const disableRotate = () => {
      if (_orbitControls) _orbitControls.enableRotate = false;
    };
    const enableRotate = () => {
      if (_orbitControls) _orbitControls.enableRotate = true;
    };

    const onPointerDown = (e: PointerEvent) => {
      if (e.button !== 0) return;
      if (_spaceHeld) return; // Space → pan, skip paint
      if (isLassoTool) {
        disableRotate();
        const local = getLocalHit(e.clientX, e.clientY);
        if (local) handleLassoClick(local);
        return;
      }
      isPainting.current = true;
      disableRotate();
      pick(e.clientX, e.clientY);
    };

    const onPointerMove = (e: PointerEvent) => {
      if (isPainting.current) {
        // Fill and eyedropper tools only pick once per click (no drag)
        if (activeTool === "fill" || activeTool === "picker") return;
        pick(e.clientX, e.clientY);
        return;
      }

      // Lasso: show rubber-band preview; highlight when cursor nears the start.
      if (isLassoTool) {
        const local = getLocalHit(e.clientX, e.clientY);
        if (!local) {
          setLassoPreview(null);
          setLassoClosing(false);
          setLassoSnap(null);
          lassoSnapRef.current = null;
          return;
        }
        setLassoPreview(local);
        const start = lassoPointsRef.current[0];
        if (start && lassoPointsRef.current.length >= 2) {
          setLassoClosing(local.distanceTo(start) < closeThreshold);
        } else {
          setLassoClosing(false);
        }
        // Snap preview: nearest local vertex to the cursor (where the next
        // click will land). Cheap brute-force over vertices; only re-render
        // when the snapped vertex actually changes.
        const verts = meshData?.vertices;
        if (verts) {
          const sv = nearestVertexLocal(verts, local);
          if (sv && (!lassoSnapRef.current || sv.distanceTo(lassoSnapRef.current) > 1e-6)) {
            lassoSnapRef.current = sv.clone();
            setLassoSnap(sv);
          }
        }
        return;
      }

      // Hover: lightweight raycaster for brush cursor preview
      if (!isBrushTool || segmentView || !geometry || !meshRef.current) {
        setHoverInfo(null);
        return;
      }

      const rect = canvas.getBoundingClientRect();
      const mouse = new THREE.Vector2(
        ((e.clientX - rect.left) / rect.width) * 2 - 1,
        -((e.clientY - rect.top) / rect.height) * 2 + 1
      );
      raycaster.setFromCamera(mouse, camera);
      const hits = raycaster.intersectObject(meshRef.current, false);
      if (hits.length > 0) {
        const hit = hits[0];
        const pos = hit.point.clone();
        // Compute face normal from geometry
        const geo = meshRef.current.geometry;
        const idx = geo.index;
        if (idx && hit.faceIndex != null) {
          const a = idx.getX(hit.faceIndex * 3);
          const b = idx.getX(hit.faceIndex * 3 + 1);
          const c = idx.getX(hit.faceIndex * 3 + 2);
          const posAttr = geo.getAttribute("position");
          const vA = new THREE.Vector3().fromBufferAttribute(posAttr, a);
          const vB = new THREE.Vector3().fromBufferAttribute(posAttr, b);
          const vC = new THREE.Vector3().fromBufferAttribute(posAttr, c);
          const normal = new THREE.Vector3()
            .crossVectors(
              vB.clone().sub(vA),
              vC.clone().sub(vA)
            )
            .normalize();
          setHoverInfo({ position: pos, normal });
        }
      } else {
        setHoverInfo(null);
      }
    };

    const onPointerUp = () => {
      if (isPainting.current) {
        isPainting.current = false;
        enableRotate();
        // Finalize segment after drag ends
        if (activeTool === "segment" && currentSegLabelRef.current !== null) {
          finalizeSegment(currentSegLabelRef.current);
          currentSegLabelRef.current = null;
          segPaintedFacesRef.current.clear();
        }
      } else if (isLassoTool) {
        enableRotate();
      }
    };

    const onPointerLeave = () => {
      if (isPainting.current) {
        isPainting.current = false;
        enableRotate();
        // Finalize segment on pointer leave too
        if (activeTool === "segment" && currentSegLabelRef.current !== null) {
          finalizeSegment(currentSegLabelRef.current);
          currentSegLabelRef.current = null;
          segPaintedFacesRef.current.clear();
        }
      }
      if (isLassoTool) {
        setLassoPreview(null);
        setLassoClosing(false);
        setLassoSnap(null);
        lassoSnapRef.current = null;
      }
      setHoverInfo(null);
    };

    canvas.addEventListener("pointerdown", onPointerDown);
    canvas.addEventListener("pointermove", onPointerMove);
    canvas.addEventListener("pointerup", onPointerUp);
    canvas.addEventListener("pointerleave", onPointerLeave);

    return () => {
      canvas.removeEventListener("pointerdown", onPointerDown);
      canvas.removeEventListener("pointermove", onPointerMove);
      canvas.removeEventListener("pointerup", onPointerUp);
      canvas.removeEventListener("pointerleave", onPointerLeave);
    };
  }, [gl.domElement, pick, activeTool, isBrushTool, isSegmentTool, isLassoTool, segmentView, geometry, raycaster, camera, getLocalHit, handleLassoClick, closeThreshold]);

  // Show lasso usage hint when the tool is selected.
  useEffect(() => {
    if (isLassoTool) setStatusMessage(t("lasso.hint"));
  }, [isLassoTool, setStatusMessage, t]);

  // Lasso keyboard:
  //   Esc          → clear the whole in-progress loop
  //   Backspace    → remove the last selected point (finer than Esc)
  //   Ctrl/Cmd+Z   → undo: pop last point if a loop is active, otherwise
  //                  revert the last finalized manual region (backend)
  useEffect(() => {
    if (!isLassoTool) return;
    const onKey = (e: KeyboardEvent) => {
      const hasLoop = lassoPointsRef.current.length > 0;
      const clearAll = () => {
        lassoPointsRef.current = [];
        setLassoPoints([]);
        lassoStartVertexRef.current = null;
        setLassoPreview(null);
        setLassoClosing(false);
        setLassoSnap(null);
        lassoSnapRef.current = null;
      };
      const popPoint = () => {
        if (!hasLoop) return;
        const next = lassoPointsRef.current.slice(0, -1);
        lassoPointsRef.current = next;
        setLassoPoints(next);
        if (next.length === 0) lassoStartVertexRef.current = null;
        setLassoClosing(false);
        setStatusMessage(
          t("lasso.undoPoint") + (next.length > 0 ? `（剩 ${next.length} 个点）` : "")
        );
      };
      if (e.key === "Escape") {
        if (hasLoop) {
          clearAll();
          setStatusMessage(t("lasso.cancelled"));
        }
        return;
      }
      if (e.key === "Backspace") {
        e.preventDefault();
        popPoint();
        return;
      }
      if ((e.ctrlKey || e.metaKey) && (e.key === "z" || e.key === "Z")) {
        e.preventDefault();
        if (hasLoop) {
          popPoint();
        } else {
          manualRegionUndo();
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [isLassoTool, setStatusMessage, t, manualRegionUndo]);

  if (!geometry) return null;

  return (
    <group rotation={[-Math.PI / 2, 0, 0]}>
      <mesh ref={meshRef} geometry={geometry} castShadow receiveShadow>
        <meshStandardMaterial vertexColors side={THREE.DoubleSide} />
      </mesh>
      <lineSegments>
        <wireframeGeometry args={[geometry]} />
        <lineBasicMaterial color={0x333333} opacity={0.1} transparent />
      </lineSegments>
      {/* Brush radius preview cursor */}
      {isBrushTool && !segmentView && (
        <BrushCursor hoverInfo={hoverInfo} brushRadius={brushRadius} color={currentColor} />
      )}
      {/* Segment boundary outline */}
      {selectedSegment !== null && meshData && meshData.segmentLabels.length > 0 && (
        <SegmentOutline meshData={meshData} selectedSegment={selectedSegment} />
      )}
      {/* Lasso overlay (manual region selection) */}
      {(lassoPoints.length > 0 || lassoPreview || lassoSnap) && (
        <LassoOverlay
          points={lassoPoints}
          preview={lassoPreview}
          closing={lassoClosing}
          dotSize={dotSize}
          snap={lassoSnap}
        />
      )}
    </group>
  );
}

// ─── Scene Lighting ───────────────────────────────────────────────
function SceneSetup() {
  return (
    <>
      <ambientLight intensity={0.4} />
      <directionalLight position={[10, 10, 5]} intensity={0.8} castShadow />
      <directionalLight position={[-5, -5, -5]} intensity={0.3} />
    </>
  );
}

// ─── Progress Bar UI ──────────────────────────────────────────────
function ProgressBar() {
  const t = useT();
  const isLoading = useAppStore((s) => s.isLoading);
  const importProgress = useAppStore((s) => s.importProgress);
  const importStage = useAppStore((s) => s.importStage);

  if (!isLoading) return null;

  const pct = Math.round(importProgress * 100);

  return (
    <div style={progressStyles.overlay}>
      <div style={progressStyles.card}>
        <div style={progressStyles.spinner}>⏳</div>
        <div style={progressStyles.stageText}>{importStage || t("view.loading")}</div>
        <div style={progressStyles.barOuter}>
          <div
            style={{
              ...progressStyles.barInner,
              width: `${pct}%`,
            }}
          />
        </div>
        <div style={progressStyles.pctText}>{pct}%</div>
      </div>
    </div>
  );
}

const progressStyles: Record<string, React.CSSProperties> = {
  overlay: {
    position: "absolute",
    top: 0, left: 0, right: 0, bottom: 0,
    background: "rgba(0,0,0,0.6)",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    zIndex: 10,
  },
  card: {
    background: "#1e1e1e",
    borderRadius: 12,
    padding: "24px 32px",
    textAlign: "center",
    minWidth: 280,
    boxShadow: "0 4px 24px rgba(0,0,0,0.5)",
  },
  spinner: {
    fontSize: 36,
    marginBottom: 12,
  },
  stageText: {
    color: "#ccc",
    fontSize: 14,
    marginBottom: 12,
    minHeight: 20,
  },
  barOuter: {
    width: "100%",
    height: 8,
    background: "#333",
    borderRadius: 4,
    overflow: "hidden",
  },
  barInner: {
    height: "100%",
    background: "linear-gradient(90deg, #4a9eff, #00d4ff)",
    borderRadius: 4,
    transition: "width 0.3s ease",
  },
  pctText: {
    color: "#4a9eff",
    fontSize: 20,
    fontWeight: 700,
    marginTop: 8,
  },
};

// ─── Controls Help Bar ────────────────────────────────────────────
function ControlsHelp() {
  const t = useT();
  return (
    <div style={helpStyles.bar}>
      <span style={helpStyles.item}>{t("controls.leftPaint")}</span>
      <span style={helpStyles.sep}>|</span>
      <span style={helpStyles.item}>{t("controls.rightRotate")}</span>
      <span style={helpStyles.sep}>|</span>
      <span style={helpStyles.item}>{t("controls.middlePan")}</span>
      <span style={helpStyles.sep}>|</span>
      <span style={helpStyles.item}>{t("controls.scrollZoom")}</span>
    </div>
  );
}

const helpStyles: Record<string, React.CSSProperties> = {
  bar: {
    position: "absolute",
    bottom: 0,
    left: 0,
    right: 0,
    background: "rgba(0,0,0,0.5)",
    color: "#aaa",
    fontSize: 12,
    padding: "6px 12px",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    gap: 4,
    zIndex: 5,
    pointerEvents: "none",
  },
  item: { whiteSpace: "nowrap" },
  sep: { color: "#555", margin: "0 4px" },
};

// ─── Segment View Toggle ──────────────────────────────────────────
function SegmentToggle() {
  const t = useT();
  const isLoaded = useAppStore((s) => s.isLoaded);
  const segmentView = useAppStore((s) => s.segmentView);
  const setSegmentView = useAppStore((s) => s.setSegmentView);
  const segments = useAppStore((s) => s.segments);

  if (!isLoaded || segments.length === 0) return null;

  return (
    <div style={segToggleStyles.container}>
      <button
        onClick={() => setSegmentView(!segmentView)}
        style={{
          ...segToggleStyles.button,
          ...(segmentView ? segToggleStyles.buttonActive : {}),
        }}
        title={segmentView ? t("view.switchToPaint") : t("view.showSegments")}
      >
        {segmentView ? t("view.paintView") : t("view.segmentView")}
      </button>
    </div>
  );
}

const segToggleStyles: Record<string, React.CSSProperties> = {
  container: {
    position: "absolute",
    top: 12,
    right: 12,
    zIndex: 5,
  },
  button: {
    padding: "6px 12px",
    border: "1px solid #555",
    borderRadius: 6,
    background: "#2d2d2d",
    color: "#ccc",
    cursor: "pointer",
    fontSize: 13,
  },
  buttonActive: {
    borderColor: "#4a9eff",
    background: "#2a3a4a",
    color: "#4a9eff",
  },
};

// ─── Viewport Root ────────────────────────────────────────────────
export function Viewport() {
  const t = useT();
  const isLoaded = useAppStore((s) => s.isLoaded);

  return (
    <div style={{ width: "100%", height: "100%", position: "relative" }}>
      <Canvas
        camera={{ position: [50, 50, 50], fov: 45 }}
        gl={{ antialias: true, preserveDrawingBuffer: true }}
        style={{ background: "#2a2a2a" }}
        onCreated={({ gl }) => {
          log.info("Viewport", "Canvas ready", {
            pixelRatio: gl.getPixelRatio(),
            size: [gl.domElement.width, gl.domElement.height],
          });
        }}
      >
        <SceneSetup />
        <CameraFit />
        <ControlsBridge />
        {isLoaded && <MeshDisplay />}
        <AdaptiveGrid />
      </Canvas>
      <ProgressBar />
      <SegmentToggle />
      <ControlsHelp />
      {!isLoaded && (
        <div
          style={{
            position: "absolute",
            top: "50%",
            left: "50%",
            transform: "translate(-50%, -50%)",
            color: "#888",
            fontSize: "18px",
            textAlign: "center",
            pointerEvents: "none",
          }}
        >
          {t("view.emptyState")}
        </div>
      )}
    </div>
  );
}
