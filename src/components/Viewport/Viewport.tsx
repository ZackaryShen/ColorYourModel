import { Canvas, useFrame, useThree } from "@react-three/fiber";
import { OrbitControls } from "@react-three/drei";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as THREE from "three";
import { useAppStore } from "../../store/appStore";
import { useMesh } from "../../hooks/useMesh";
import { usePaintTool, WHOLE_SEGMENT_MAX_SHARE, MANUAL_SEGMENT_OFFSET } from "../../hooks/usePaintTool";
import { useTauriCommand } from "../../hooks/useTauriCommand";
import { log } from "../../utils/logger";
import { useT } from "../../i18n";
import { computeBoundsTree, disposeBoundsTree, acceleratedRaycast } from "three-mesh-bvh";

// Accelerated raycasting via a bounding-volume hierarchy (three-mesh-bvh).
// Patched ONCE at module load. `acceleratedRaycast` falls back to the native
// raycast when `geometry.boundsTree` is absent, so this is safe even before the
// tree is built (iteration 16). `indirect: true` is REQUIRED for our non-indexed
// render geometry: it keeps `geometry.index === null` and reports the ORIGINAL
// triangle order, so `intersection.faceIndex` still equals the backend face
// index (a plain `computeBoundsTree()` would physically reorder the index and
// silently revive the "paint at A, color at B" bug from iteration 14).
THREE.BufferGeometry.prototype.computeBoundsTree = computeBoundsTree;
THREE.BufferGeometry.prototype.disposeBoundsTree = disposeBoundsTree;
THREE.Mesh.prototype.raycast = acceleratedRaycast;

// ─── Theme-aware 3D overlay colours (iteration 21) ────────────────
// Until iteration 20 the 3D canvas was locked to #2a2a2a in both themes, so
// every overlay could hard-code a neon colour tuned for a dark background.
// Iteration 21 lets `--bg-canvas` follow the theme (#eef1f5 in light), which
// drops those neons to 1.1–1.3:1 — effectively invisible. Each entry below is
// the WCAG-checked light-mode counterpart (contrast measured against #eef1f5):
//   highlight  #b45309  4.43:1   (was #ffee00 / #ffff00 → 1.09:1)
//   cursor     #0088aa  3.63:1   (was #00ffff          → 1.25:1)
//   lassoStart #0b8043  4.43:1   (was #00ff88          → 1.18:1, REFUTE M2)
// `accent` (#4a9eff) is intentionally left alone: it is the same token the CSS
// chrome uses in both themes, and REFUTE did not flag it.
// Also deliberately unchanged: the grid (#555555/#333333 already score
// 6.6:1/11.2:1 on the light canvas — REFUTE M1) and the 0.05-opacity wireframe.
const OVERLAY_COLORS = {
  dark: {
    highlight: 0xffee00,
    outline: 0xffff00,
    cursor: 0x00ffff,
    lassoStart: 0x00ff88,
  },
  light: {
    highlight: 0xb45309,
    outline: 0xb45309,
    cursor: 0x0088aa,
    lassoStart: 0x0b8043,
  },
} as const;

/** Subscribes to the theme from inside the R3F tree. zustand does not rely on
 *  React context, so this works across the reconciler boundary. */
function useOverlayColors() {
  const theme = useAppStore((s) => s.theme);
  return theme === "light" ? OVERLAY_COLORS.light : OVERLAY_COLORS.dark;
}

// Shared controls reference — set by ControlsBridge, read by MeshDisplay
let _orbitControls: (THREE.EventDispatcher & {
  enableRotate: boolean;
  enablePan: boolean;
  mouseButtons: { LEFT: number; MIDDLE: number; RIGHT: number };
  target: THREE.Vector3;
  update: () => void;
}) | null = null;

// Is Alt key currently held? When held, left-drag rotates the camera even
// while a paint/segment/lasso tool is active (see MeshDisplay Alt handler).
let _altHeld = false;

// Tracks whether the cursor is currently over the model surface. Drives the
// context-sensitive LEFT mapping (paint on model vs rotate on empty space,
// iteration 15, req #1). Module-level so BOTH ControlsBridge (tool change)
// and MeshDisplay (hover change) read the same value — single source of truth.
const overModelRef = { current: false };

// Sentinel for OrbitControls.mouseButtons.LEFT meaning "no orbit action".
// three's onMouseDown falls through to STATE.NONE for any value that is not
// MOUSE.ROTATE/PAN/DOLLY (verified against three-stdlib OrbitControls).
const MOUSE_NONE = -1 as unknown as THREE.MOUSE;

// Shared up-axis for brush-ring orientation (module-level, reused every frame —
// avoids per-frame Vector3 allocation in BrushCursorImperative, iteration 16).
const UP_Z = new THREE.Vector3(0, 0, 1);

// SINGLE OWNER of OrbitControls' mouse-button mapping. View tool: LEFT=rotate.
// Paint-like tools (brush/segment/fill/picker): LEFT = paint when over the
// model, LEFT = rotate when over empty space (so a drag on empty orbits, not
// paints). RIGHT = pan always; MIDDLE = dolly(zoom). Lasso: LEFT disabled
// when over model (our handler places points), LEFT = rotate when over empty
// space (iteration 22: user expects off-model drag to orbit). Alt always forces
// ROTATE regardless of tool (iteration 22 fix: Alt was dead code — _altHeld
// was set but never consumed here).
// Called from the tool-change effect AND the hover-change path so the mapping
// is never stale (iteration 15, req #1).
function applyCameraButtons() {
  const c = _orbitControls;
  if (!c) return;
  const tool = useAppStore.getState().activeTool;
  const over = overModelRef.current;
  // Alt overrides everything: user explicitly wants to rotate.
  if (_altHeld) { c.mouseButtons = { LEFT: THREE.MOUSE.ROTATE, MIDDLE: THREE.MOUSE.DOLLY, RIGHT: THREE.MOUSE.PAN }; return; }
  let left: THREE.MOUSE;
  if (tool === "view") left = THREE.MOUSE.ROTATE;
  else if (tool === "lasso") left = over ? MOUSE_NONE : THREE.MOUSE.ROTATE;
  else left = over ? MOUSE_NONE : THREE.MOUSE.ROTATE;
  c.mouseButtons = { LEFT: left, MIDDLE: THREE.MOUSE.DOLLY, RIGHT: THREE.MOUSE.PAN };
}

// ─── Face Picker Hook (raycaster-based) ─────────────────────────
// Raycasts the mesh and returns the 0-based face (triangle) index under the
// cursor. The NDC is derived purely from the canvas bounding rect, which is
// resolution- and DPR-independent. This removes the GPU pixel-pick fragility
// where the read-back pixel had to exactly match `size * dpr`: whenever
// `rect` (actual canvas bbox) diverged from `size` (R3F parent measure) — HiDPI
// scaling, CSS transforms, a render target created before `size` settled, or a
// dpr change — the read pixel shifted off-cursor and paint landed elsewhere
// ("mouse does not follow"). It reuses the same robust path the lasso tool
// uses (getLocalHit), only keeping the integer face index here.
function useFacePicker(
  meshRef: React.RefObject<THREE.Mesh | null>,
  onFacePicked: (faceId: number) => void
) {
  const { gl, camera } = useThree();
  const raycaster = useMemo(() => {
    const r = new THREE.Raycaster();
    // BVH: return only the nearest hit (we only ever read hits[0]).
    r.firstHitOnly = true;
    return r;
  }, []);

  const pick = useCallback(
    (clientX: number, clientY: number) => {
      const mesh = meshRef.current;
      if (!mesh) return;
      const rect = gl.domElement.getBoundingClientRect();
      const mouse = new THREE.Vector2(
        ((clientX - rect.left) / rect.width) * 2 - 1,
        -((clientY - rect.top) / rect.height) * 2 + 1
      );
      raycaster.setFromCamera(mouse, camera);
      const hits = raycaster.intersectObject(mesh, false);
      if (hits.length === 0 || hits[0].faceIndex == null) return;
      onFacePicked(hits[0].faceIndex);
    },
    [gl, camera, meshRef, raycaster, onFacePicked]
  );

  return pick;
}

// ─── Camera Auto-Fit ──────────────────────────────────────────────
function CameraFit() {
  const { camera, size } = useThree();
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

    // Orthographic projection: a pan is a pure parallel slide with ZERO parallax,
    // so the view plane stays fixed during panning (explicit user requirement).
    // Under ortho the camera-to-model distance only affects near/far clipping,
    // never apparent size — so we place it at a safe distance for the frustum.
    const cam = camera as THREE.OrthographicCamera;
    // Slight iso tilt aids depth readability. Under ortho this introduces NO
    // parallax, so panning remains a perfectly flat screen translation.
    const dir = new THREE.Vector3(0.4, 0.4, 1).normalize();
    const viewDist = maxDim * 10;
    cam.position.set(cx + dir.x * viewDist, cy + dir.y * viewDist, cz + dir.z * viewDist);
    cam.lookAt(cx, cy, cz);

    // Fit into the viewport. The effective dimension must cover BOTH the model
    // geometry AND the AdaptiveGrid helper (which spans ~2.5× the model's XZ
    // footprint). Without this, the grid overflows the frustum and its bottom/
    // edges get clipped on initial load (iteration 21, issue: "起始的网格缺失").
    // Using 1.3× the XZ span keeps most grid visible while not wasting space.
    const spanXZ = Math.max(dimX, dimZ);
    const effectiveDim = Math.max(maxDim, spanXZ * 1.3);
    const fit = Math.min(size.width, size.height) / effectiveDim;
    cam.zoom = Math.min(fit * 0.88, 500); // 0.88 fills ~88% of viewport; clamped to maxZoom (iteration 22)
    cam.near = Math.max(0.1, viewDist - maxDim * 2);
    cam.far = viewDist + maxDim * 2;
    cam.updateProjectionMatrix();

    // CRITICAL: OrbitControls orbits and pans around its `target`. If we only
    // move the camera but leave target at the default (0,0,0), the pivot is
    // offset from the model center. Point the pivot at the model center so
    // orbit/pan feel natural.
    const syncTarget = () => {
      if (_orbitControls) {
        _orbitControls.target.set(cx, cy, cz);
        _orbitControls.update();
      }
    };
    if (_orbitControls) {
      syncTarget();
    } else {
      // ControlsBridge may mount its effect after this one, leaving
      // _orbitControls null on first run. Retry on the next frame so the pivot
      // is still synced on initial model load.
      requestAnimationFrame(syncTarget);
    }

    log.info("CameraFit", "Camera positioned (ortho)", {
      center: [cx, cy, cz],
      maxDim,
      effectiveDim: +effectiveDim.toFixed(1),
      zoom: +cam.zoom.toFixed(3),
      viewDist: +viewDist.toFixed(1),
    });
    // Depend on the BBOX, not the whole `meshData` object. Every partition /
    // undo / redo operation replaces `meshData` with a shallow copy (same bbox
    // reference), which re-ran this effect and snapped the camera back to its
    // initial framing — "Undo/Redo 会刷新视图" and "分区画笔点一下就回初始视图"
    // (iteration 18, Issues 4 & 5). The bbox only changes on a real mesh load,
    // which is exactly when an auto-fit IS wanted.
  }, [meshData?.bbox, camera, size]);

  return null;
}

// ─── Controls Bridge ──────────────────────────────────────────────
// Camera control mapping. Matches the i18n help text (left=rotate, right=pan, middle=zoom).
// Assigned imperatively on the controls instance so React re-renders never re-apply
// a fresh mouseButtons object and wipe the Space-hold LEFT=PAN override used
// by the lasso/paint handlers.
function ControlsBridge() {
  const ref = useRef<any>(null);
  const activeTool = useAppStore((s) => s.activeTool);

  useEffect(() => {
    _orbitControls = ref.current;
    // Apply the (tool + over-model) button mapping now that the controls exist.
    applyCameraButtons();
    // Diagnostic: log camera state around right-drag (pan) operations to
    // distinguish true rotation from perspective-parallax apparent rotation.
    const el = ref.current?.domElement;
    if (el) {
      const onDown = (e: PointerEvent) => {
        if (e.button === 2 && _orbitControls) {
          const c = (_orbitControls as unknown as { object: THREE.Camera }).object;
          const t = _orbitControls.target;
          log.info("PanDiag", "RIGHT-down", {
            pos: [c.position.x.toFixed(2), c.position.y.toFixed(2), c.position.z.toFixed(2)],
            tgt: [t.x.toFixed(2), t.y.toFixed(2), t.z.toFixed(2)],
            quat: [c.quaternion.x.toFixed(4), c.quaternion.y.toFixed(4), c.quaternion.z.toFixed(4), c.quaternion.w.toFixed(4)],
          });
        }
      };
      const onUp = (e: PointerEvent) => {
        if (e.button === 2 && _orbitControls) {
          const c = (_orbitControls as unknown as { object: THREE.Camera }).object;
          const t = _orbitControls.target;
          log.info("PanDiag", "RIGHT-up", {
            pos: [c.position.x.toFixed(2), c.position.y.toFixed(2), c.position.z.toFixed(2)],
            tgt: [t.x.toFixed(2), t.y.toFixed(2), t.z.toFixed(2)],
            quat: [c.quaternion.x.toFixed(4), c.quaternion.y.toFixed(4), c.quaternion.z.toFixed(4), c.quaternion.w.toFixed(4)],
          });
        }
      };
      el.addEventListener("pointerdown", onDown);
      el.addEventListener("pointerup", onUp);
      return () => {
        el.removeEventListener("pointerdown", onDown);
        el.removeEventListener("pointerup", onUp);
      };
    }
    return () => { _orbitControls = null; };
  }, []);

  // Mouse-button mapping is owned by the single `applyCameraButtons()` helper
  // (module scope). It maps LEFT context-sensitively (paint on model vs rotate
  // on empty space) using the shared `overModelRef`. Called here on tool change
  // AND from the hover path in MeshDisplay, so there is exactly one writer and
  // the mapping never goes stale (iteration 15, req #1).
  useEffect(() => {
    applyCameraButtons();
  }, [activeTool]);

  return (
    <OrbitControls
      ref={ref}
      makeDefault
      enableRotate
      enablePan
      enableZoom
      // Orthographic camera: wheel/dolly adjusts `camera.zoom` instead of moving
      // the camera. Clamp it so the model can't be zoomed into the void.
      // 500× allows inspecting sub-millimeter features on mm-scale prints (iteration 22:
      // was 50, too restrictive for detailed work).
      minZoom={0.1}
      maxZoom={500}
      // REFUTE-driven fix (iteration 7, problem 1): drei defaults
      // enableDamping=true, which keeps applying camera inertia every frame.
      // After a left-drag rotate, that leftover angular velocity is still
      // integrated while the user right-drag PANS, reading as "extra rotation"
      // during pan. Disabling damping makes every drag stop exactly on release
      // — pan no longer carries a phantom orbit.
      enableDamping={false}
      screenSpacePanning
    />
  );
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

/// Imperative brush cursor driven by useFrame + ref — reads hoverInfoRef each
/// frame at GPU rate. Eliminates 60–120Hz React reconciliation that the old
/// props-based BrushCursor caused on every pointermove (the #2 perf hotspot
/// after dual raycast). The ref is stable; only its .current content mutates.
function BrushCursorImperative({ hoverInfoRef, brushRadius, color }: {
  hoverInfoRef: React.MutableRefObject<HoverInfo | null>;
  brushRadius: number;
  color: [number, number, number, number];
}) {
  const meshRef = useRef<THREE.Mesh>(null);

  useFrame(() => {
    const info = hoverInfoRef.current;
    const m = meshRef.current;
    if (!info || !m) {
      if (m) m.visible = false;
      return;
    }
    m.visible = true;
    m.position.copy(info.position);
    // Orient ring to face the hit normal. setFromUnitVectors writes directly into
    // m.quaternion — no per-frame temp allocation (iteration 16, REFUTE minor-5).
    m.quaternion.setFromUnitVectors(UP_Z, info.normal);
  });

  const ringColor = new THREE.Color(color[0] / 255, color[1] / 255, color[2] / 255);

  return (
    <mesh ref={meshRef} visible={false}>
      <ringGeometry args={[brushRadius * 0.9, brushRadius, 32]} />
      <meshBasicMaterial
        color={ringColor}
        transparent
        opacity={0.5}
        side={THREE.DoubleSide}
        depthTest={false}
      />
    </mesh>
  );
}

// ─── Fill Face Highlight (hovered face outline for the fill tool) ──
/// When the fill tool is active, the face under the cursor is marked with a
/// bright triangle OUTLINE so the user sees exactly which face a click will
/// target — the requested "面片高亮". Drawn as an OUTLINE (not a solid fill):
/// fill actually floods a whole partition or connected region, so a solid
/// highlight would LIE about the scope (REFUTE M2). The outline reads as
/// "the face you are pointing at", which is honest.
/// Geometry is created ONCE (stable reference, passed via `geometry={}`, never
/// through JSX `args`) so R3F never reconstructs it and wipes our in-place
/// vertex writes (REFUTE B2). `frustumCulled={false}` stops the zero-initialized
/// bounding sphere from culling the mesh out of view (REFUTE B1).
function FillFaceHighlight({ faceRef }: { faceRef: React.MutableRefObject<THREE.Vector3[] | null> }) {
  const overlay = useOverlayColors();
  const objRef = useRef<THREE.LineLoop>(null);
  const geometry = useMemo(() => {
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.BufferAttribute(new Float32Array(9), 3));
    return g;
  }, []);

  useFrame(() => {
    const obj = objRef.current;
    if (!obj) return;
    const tri = faceRef.current;
    if (!tri || tri.length !== 3) {
      obj.visible = false;
      return;
    }
    obj.visible = true;
    const pos = geometry.getAttribute("position") as THREE.BufferAttribute;
    pos.setXYZ(0, tri[0].x, tri[0].y, tri[0].z);
    pos.setXYZ(1, tri[1].x, tri[1].y, tri[1].z);
    pos.setXYZ(2, tri[2].x, tri[2].y, tri[2].z);
    pos.needsUpdate = true;
  });

  return (
    <lineLoop ref={objRef} geometry={geometry} frustumCulled={false} renderOrder={4}>
      <lineBasicMaterial color={overlay.highlight} depthTest={false} transparent opacity={0.95} />
    </lineLoop>
  );
}

// ─── Adjacency helpers (perf: precompute once per meshData) ─────
interface MeshDataLike {
  vertices: number[];
  faces: number[];
  segmentLabels: number[];
}

/// Build an edge→faces map with a NUMERIC key `a*vmax + b` (a<b). Numeric keys
/// avoid the per-edge string allocation of the old `"a_b"` Map (the previous
/// source of selection lag on large meshes) while still keeping ALL co-faces in
/// the list — so non-manifold edges (3+ shared faces) are detected correctly.
function buildEdgeMap(meshData: MeshDataLike): { map: Map<number, number[]>; vmax: number } {
  const faces = meshData.faces;
  const vmax = meshData.vertices.length / 3 + 1; // > any vertex index
  const map = new Map<number, number[]>();
  const faceCount = faces.length / 3;
  for (let f = 0; f < faceCount; f++) {
    const v0 = faces[f * 3], v1 = faces[f * 3 + 1], v2 = faces[f * 3 + 2];
    const edges = [
      [Math.min(v0, v1), Math.max(v0, v1)],
      [Math.min(v1, v2), Math.max(v1, v2)],
      [Math.min(v2, v0), Math.max(v2, v0)],
    ];
    for (const [a, b] of edges) {
      const key = a * vmax + b;
      const list = map.get(key);
      if (list) list.push(f);
      else map.set(key, [f]);
    }
  }
  return { map, vmax };
}

/// Group face indices by their segment label (one pass over all faces).
function buildFacesBySegment(meshData: MeshDataLike): Map<number, number[]> {
  const faces = meshData.faces;
  const labels = meshData.segmentLabels;
    const map = new Map<number, number[]>();
  const faceCount = faces.length / 3;
  for (let f = 0; f < faceCount; f++) {
    const label = labels[f];
    const list = map.get(label);
    if (list) list.push(f);
    else map.set(label, [f]);
  }
  return map;
}

/// Vertex→incident-faces map + averaged per-vertex normals (one pass over
/// all faces). Powers the same-side lasso snap preview (frontend mirror of the
/// backend `snap_point_to_vertex_on_face` same-side test). Extracted from the
/// old inline `useMemo` so it can be computed in a deferred effect.
function buildVertexData(meshData: MeshDataLike): {
  vertexFaces: Map<number, number[]>;
  vertexNormals: Float32Array;
} {
  const faces = meshData.faces;
  const verts = meshData.vertices;
  const vmax = verts.length / 3;
  const vf = new Map<number, number[]>();
  const faceCount = faces.length / 3;
  for (let f = 0; f < faceCount; f++) {
    const a = faces[f * 3], b = faces[f * 3 + 1], c = faces[f * 3 + 2];
    for (const v of [a, b, c]) {
      const list = vf.get(v);
      if (list) list.push(f);
      else vf.set(v, [f]);
    }
  }
  const vn = new Float32Array(verts.length);
  const faceNormal = (f: number): [number, number, number] => {
    const a = faces[f * 3], b = faces[f * 3 + 1], c = faces[f * 3 + 2];
    const ax = verts[a * 3], ay = verts[a * 3 + 1], az = verts[a * 3 + 2];
    const bx = verts[b * 3], by = verts[b * 3 + 1], bz = verts[b * 3 + 2];
    const cx = verts[c * 3], cy = verts[c * 3 + 1], cz = verts[c * 3 + 2];
    const e1x = bx - ax, e1y = by - ay, e1z = bz - az;
    const e2x = cx - ax, e2y = cy - ay, e2z = cz - az;
    return [e1y * e2z - e1z * e2y, e1z * e2x - e1x * e2z, e1x * e2y - e1y * e2x];
  };
  for (let f = 0; f < faceCount; f++) {
    const n = faceNormal(f);
    const len = Math.hypot(n[0], n[1], n[2]) || 1;
    const nx = n[0] / len, ny = n[1] / len, nz = n[2] / len;
    for (const v of [faces[f * 3], faces[f * 3 + 1], faces[f * 3 + 2]]) {
      vn[v * 3] += nx; vn[v * 3 + 1] += ny; vn[v * 3 + 2] += nz;
    }
  }
  for (let i = 0; i < vmax; i++) {
    const x = vn[i * 3], y = vn[i * 3 + 1], z = vn[i * 3 + 2];
    const l = Math.hypot(x, y, z) || 1;
    vn[i * 3] = x / l; vn[i * 3 + 1] = y / l; vn[i * 3 + 2] = z / l;
  }
  return { vertexFaces: vf, vertexNormals: vn };
}

// ─── Segment Outline (boundary edges for selected segment) ────────
function SegmentOutline({ meshData, selectedSegment, edgeMap, facesBySeg }: {
  meshData: { faces: number[]; segmentLabels: number[]; vertices: number[] };
  selectedSegment: number;
  edgeMap: { map: Map<number, number[]>; vmax: number };
  facesBySeg: Map<number, number[]>;
}) {
  const overlay = useOverlayColors();
  const geometry = useMemo(() => {
    const faces = meshData.faces;
    const labels = meshData.segmentLabels;
    const verts = meshData.vertices;
    const { map: edgeToFaces, vmax } = edgeMap;

    // Only iterate the faces belonging to the selected segment (cheap even when
    // switching selection on a huge mesh). Boundary = an edge whose co-face has
    // a different label, or an edge belonging to a single face (mesh border).
    const selectedFaces = facesBySeg.get(selectedSegment);
    if (!selectedFaces || selectedFaces.length === 0) return null;

    const boundaryVerts: number[] = [];
    for (const f of selectedFaces) {
      const v0 = faces[f * 3], v1 = faces[f * 3 + 1], v2 = faces[f * 3 + 2];
      const edges = [
        [Math.min(v0, v1), Math.max(v0, v1)],
        [Math.min(v1, v2), Math.max(v1, v2)],
        [Math.min(v2, v0), Math.max(v2, v0)],
      ];
      for (const [a, b] of edges) {
        const cofaces = edgeToFaces.get(a * vmax + b);
        if (!cofaces) continue;
        const onBoundary =
          cofaces.some((f2) => labels[f2] !== selectedSegment) || cofaces.length === 1;
        if (onBoundary) {
          boundaryVerts.push(
            verts[a * 3], verts[a * 3 + 1], verts[a * 3 + 2],
            verts[b * 3], verts[b * 3 + 1], verts[b * 3 + 2]
          );
        }
      }
    }

    if (boundaryVerts.length === 0) return null;

    const geo = new THREE.BufferGeometry();
    geo.setAttribute("position", new THREE.Float32BufferAttribute(boundaryVerts, 3));
    return geo;
  }, [meshData.faces, meshData.segmentLabels, meshData.vertices, selectedSegment, edgeMap, facesBySeg]);

  if (!geometry) return null;

  return (
    <lineSegments geometry={geometry}>
      <lineBasicMaterial color={overlay.outline} linewidth={2} depthTest={false} />
    </lineSegments>
  );
}

// ─── Segment Highlight (filled translucent overlay for selected segment) ─
function SegmentHighlight({ meshData, selectedSegment, facesBySeg }: {
  meshData: { faces: number[]; segmentLabels: number[]; vertices: number[] };
  selectedSegment: number;
  facesBySeg: Map<number, number[]>;
}) {
  const overlay = useOverlayColors();
  const geometry = useMemo(() => {
    const faces = meshData.faces;
    const verts = meshData.vertices;
    // Only the selected segment's faces — O(selected faces), not all faces.
    const selectedFaces = facesBySeg.get(selectedSegment);
    if (!selectedFaces || selectedFaces.length === 0) return null;
    const pos: number[] = [];
    for (const f of selectedFaces) {
      const a = faces[f * 3], b = faces[f * 3 + 1], c = faces[f * 3 + 2];
      pos.push(
        verts[a * 3], verts[a * 3 + 1], verts[a * 3 + 2],
        verts[b * 3], verts[b * 3 + 1], verts[b * 3 + 2],
        verts[c * 3], verts[c * 3 + 1], verts[c * 3 + 2]
      );
    }
    const g = new THREE.BufferGeometry();
    g.setAttribute("position", new THREE.Float32BufferAttribute(new Float32Array(pos), 3));
    g.computeVertexNormals();
    return g;
  }, [meshData.faces, meshData.vertices, selectedSegment, facesBySeg]);

  if (!geometry) return null;

  return (
    <mesh geometry={geometry} renderOrder={2}>
      <meshBasicMaterial
        color={overlay.cursor}
        transparent
        opacity={0.35}
        depthTest={false}
        depthWrite={false}
        side={THREE.DoubleSide}
      />
    </mesh>
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

// ─── Nearest same-side vertex (lasso snap, frontend preview) ─────
/// Mirrors the backend `snap_point_to_vertex_on_face` (iteration 12):
/// three-tier candidate restriction with local snap radius k×maxEdge(hit_face).
/// Tier 1: 1-ring same-side within threshold.  Tier 2: hit-face-only same-side.
/// Tier 3: hit-face-only any vertex.  Never falls back to global nearest.
/// The preview dot lands EXACTLY where the committed point lands (same formula).
const SNAP_K = 1.5; // shared with backend manual.rs — keep in sync

function nearestVertexLocalOnFace(
  vertices: number[],
  faces: number[],
  p: THREE.Vector3,
  faceIndex: number,
  faceNormal: THREE.Vector3,
  vertexFaces: Map<number, number[]>,
  vertexNormals: Float32Array
): THREE.Vector3 | null {
  if (faceIndex < 0 || !vertexFaces || vertexNormals.length === 0) {
    return nearestVertexLocal(vertices, p);
  }
  const fi3 = faceIndex * 3;
  const hv0 = faces[fi3], hv1 = faces[fi3 + 1], hv2 = faces[fi3 + 2];

  // Local snap radius: k × longest edge of hit face (auto-adapts to density).
  const e01sq = (vertices[hv0*3]-vertices[hv1*3])**2 + (vertices[hv0*3+1]-vertices[hv1*3+1])**2 + (vertices[hv0*3+2]-vertices[hv1*3+2])**2;
  const e12sq = (vertices[hv1*3]-vertices[hv2*3])**2 + (vertices[hv1*3+1]-vertices[hv2*3+1])**2 + (vertices[hv1*3+2]-vertices[hv2*3+2])**2;
  const e20sq = (vertices[hv2*3]-vertices[hv0*3])**2 + (vertices[hv2*3+1]-vertices[hv0*3+1])**2 + (vertices[hv2*3+2]-vertices[hv0*3+2])**2;
  const maxSnapSq = Math.max(e01sq, e12sq, e20sq) * SNAP_K * SNAP_K;

  // Candidate sets
  const faceOnly = [hv0, hv1, hv2];
  const oneRing = new Set<number>(faceOnly);
  for (const v of faceOnly) {
    const inc = vertexFaces.get(v);
    if (inc) for (const f of inc) {
      oneRing.add(faces[f * 3]);
      oneRing.add(faces[f * 3 + 1]);
      oneRing.add(faces[f * 3 + 2]);
    }
  }

  // Same-side normal threshold for lasso vertex snap (must match
  // backend manual.rs snap_point_to_vertex_on_face).  60° (was 50°) —
  // covers cube-corner case arccos(1/√3)≈54.74° with margin.
  const SNAP_COS_THRESH = Math.cos((60 * Math.PI) / 180);
  const fnx = faceNormal.x, fny = faceNormal.y, fnz = faceNormal.z;

  // Pick nearest same-side from a candidate list → (pos, sqDist) or null
  const pickSameSide = (cands: Iterable<number>): [THREE.Vector3, number] | null => {
    let best: THREE.Vector3 | null = null; let bestSq = Infinity;
    for (const vi of cands) {
      const nx = vertexNormals[vi*3], ny = vertexNormals[vi*3+1], nz = vertexNormals[vi*3+2];
      if (nx*fnx + ny*fny + nz*fnz < SNAP_COS_THRESH) continue;
      const dx = vertices[vi*3] - p.x, dy = vertices[vi*3+1] - p.y, dz = vertices[vi*3+2] - p.z;
      const sq = dx*dx + dy*dy + dz*dz;
      if (sq < bestSq) { bestSq = sq; best = new THREE.Vector3(vertices[vi*3], vertices[vi*3+1], vertices[vi*3+2]); }
    }
    return best ? [best, bestSq] : null;
  };

  // Pick nearest ANY from candidates (no normal check)
  const pickAny = (cands: Iterable<number>): THREE.Vector3 | null => {
    let best: THREE.Vector3 | null = null; let bestSq = Infinity;
    for (const vi of cands) {
      const dx = vertices[vi*3] - p.x, dy = vertices[vi*3+1] - p.y, dz = vertices[vi*3+2] - p.z;
      const sq = dx*dx + dy*dy + dz*dz;
      if (sq < bestSq) { bestSq = sq; best = new THREE.Vector3(vertices[vi*3], vertices[vi*3+1], vertices[vi*3+2]); }
    }
    return best;
  };

  // ── Tier 1: 1-ring same-side within local threshold ───────────
  const t1 = pickSameSide(oneRing);
  if (t1 && t1[1] <= maxSnapSq) return t1[0];

  // ── Tier 2: hit-face-only same-side ──────────────────────────
  const t2 = pickSameSide(faceOnly);
  if (t2) return t2[0];

  // ── Tier 3: hit-face-only any vertex (degenerate fallback) ────
  const t3 = pickAny(faceOnly);
  if (t3) return t3;

  // Absolute last resort: global nearest (bad face data only).
  return nearestVertexLocal(vertices, p);
}

/// Result of a raycast hit on the mesh surface in model-local coordinates.
interface LocalHit {
  /** Hit point in model-local coords (same space as meshData.vertices) */
  point: THREE.Vector3;
  /** Index of the intersected triangle (faceIndex * 3 = first index in geometry.index) */
  faceIndex: number;
  /** Geometric normal of the hit triangle, in model-local coords. */
  normal: THREE.Vector3;
}

// ─── Lasso Overlay (manual region selection) ────────────────────
function LassoOverlay({ points, preview, closing, dotSize, snap }: {
  points: THREE.Vector3[];
  preview: THREE.Vector3 | null;
  closing: boolean;
  dotSize: number;
  snap?: THREE.Vector3 | null;
}) {
  // Hook must run before the early return so the hook order stays stable.
  const overlay = useOverlayColors();
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

  // All markers use the same sphere shape for visual consistency.
  // Confirmed = solid blue spheres; start point = green (yellow when closing);
  // snap preview = semi-transparent cyan (clearly "not yet placed").
  const r = dotSize;

  return (
    <group>
      {segPos.length > 0 && (
        <lineSegments>
          <bufferGeometry>
            <bufferAttribute attach="attributes-position" args={[new Float32Array(segPos), 3]} />
          </bufferGeometry>
          <lineBasicMaterial
            color={closing ? overlay.outline : 0x4a9eff}
            linewidth={2}
            depthTest={false}
          />
        </lineSegments>
      )}
      {/* Confirmed selected points — uniform small spheres */}
      {points.map((p, idx) => (
        <mesh key={idx} position={p}>
          <sphereGeometry args={[r, 10, 10]} />
          <meshBasicMaterial
            color={idx === 0 ? (closing ? overlay.outline : overlay.lassoStart) : 0x4a9eff}
            depthTest={false}
          />
        </mesh>
      ))}
      {/* Snap preview — semi-transparent cyan, clearly "not yet confirmed" */}
      {snap && (
        <mesh position={snap}>
          <sphereGeometry args={[r * 1.1, 10, 10]} />
          <meshBasicMaterial color={overlay.cursor} transparent opacity={0.5} depthTest={false} />
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
  const hoveredSegment = useAppStore((s) => s.hoveredSegment);
  const setHoveredSegment = useAppStore((s) => s.setHoveredSegment);
  const activeTool = useAppStore((s) => s.activeTool);
  const brushRadius = useAppStore((s) => s.brushRadius);
  const shadingMode = useAppStore((s) => s.shadingMode);
  const currentColor = useAppStore((s) => s.currentColor);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const setLastPaintDebug = useAppStore((s) => s.setLastPaintDebug);
  const { buildGeometry, updateFaceColors } = useMesh();
  const { paintFace } = usePaintTool();
  const { paintSegmentFace, finalizeSegment, manualRegionAddPoint, finalizeManualRegion, manualRegionUndo, undoPaint, redoPaint } = useTauriCommand();
  const pushUndo = useAppStore((s) => s.pushUndo);
  const t = useT();
  const meshRef = useRef<THREE.Mesh>(null);
  const isPainting = useRef(false);
  // Segment paint state: track current label + painted faces for dedup
  const currentSegLabelRef = useRef<number | null>(null);
  const segPaintedFacesRef = useRef<Set<number>>(new Set());
  const { gl, camera } = useThree();
  const raycaster = useMemo(() => {
    const r = new THREE.Raycaster();
    // BVH: return only the nearest hit (we only ever read hits[0]).
    r.firstHitOnly = true;
    return r;
  }, []);
  // Imperative ref for brush cursor position — avoids 60–120Hz React
  // re-renders when the mouse moves over the mesh. useFrame in BrushCursor
  // reads this ref each frame at GPU rate, zero GC pressure from state cycles.
  const hoverInfoRef = useRef<HoverInfo | null>(null);

  // Fill tool: 3 model-local vertices of the face under the cursor, written by
  // the hover raycast and read by FillFaceHighlight (面片高亮, iteration 17).
  const fillHoverFaceRef = useRef<THREE.Vector3[] | null>(null);

  // Tools that show brush cursor
  const isBrushTool = activeTool === "brush" || activeTool === "spray" ||
    activeTool === "smart" || activeTool === "eraser";
  const isSegmentTool = activeTool === "segment";
  // Tools whose action is bounded by `brushRadius` and therefore need the radius
  // ring preview + Ctrl+wheel resize.
  // NOTE: Fill tool intentionally excluded — fill is a click-to-flood operation
  // (whole partition or connected region); its visual feedback is the
  // FillFaceHighlight triangle outline, not a radius ring (iteration 21).
  const isRadiusTool = isBrushTool;

  // Lasso (manual region) state
  const isLassoTool = activeTool === "lasso";
  // View/Navigate mode: left button rotates the camera (no tool action).
  const isViewTool = activeTool === "view";
  // Tools that mutate per-face paint state → snapshot for undo on stroke start.
  // Eyedropper (picker) only reads, so it is deliberately excluded.
  const modifiesFaces = isBrushTool || isSegmentTool || activeTool === "fill";
  const [lassoPoints, setLassoPoints] = useState<THREE.Vector3[]>([]);
  const [lassoPreview, setLassoPreview] = useState<THREE.Vector3 | null>(null);
  const [lassoClosing, setLassoClosing] = useState(false);
  const [lassoSnap, setLassoSnap] = useState<THREE.Vector3 | null>(null);
  const lassoSnapRef = useRef<THREE.Vector3 | null>(null);
  const lassoPointsRef = useRef<THREE.Vector3[]>([]);
  const lassoFaceIndicesRef = useRef<number[]>([]);
  const lassoStartVertexRef = useRef<number | null>(null);
  const lassoStartPointRef = useRef<THREE.Vector3 | null>(null);

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
    return Math.max(0.03, Math.sqrt(dx * dx + dy * dy + dz * dz) * 0.0008);
  }, [meshData?.bbox]);

  // The faint wireframe overlay is built from a WireframeGeometry that, for large
  // meshes, synchronously allocates a ~360k-element array with millions of string
  // concatenations (three's edge de-dup) — the REAL cause of the multi-second
  // "import lag before I can pick" (iteration 16, REFUTE blocker-2). It is nearly
  // invisible at opacity 0.05, so we simply skip it above 50k faces — a zero-risk,
  // highest-ROI fix that removes the import freeze.
  const showWireframe = !!meshData && meshData.faceCount <= 50000;

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

  // Clear the fill face-highlight when leaving the fill tool so a stale
  // triangle never lingers after a tool switch (iteration 17, REFUTE M1/P7).
  useEffect(() => {
    if (activeTool !== "fill") fillHoverFaceRef.current = null;
  }, [activeTool]);

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
  }, [buildGeometry]);

  // Build the raycasting BVH for this geometry. Deferred to a macrotask (NOT in
  // the render-phase useMemo above) so the first paint + surface picking are
  // never blocked by the O(F log F) tree build — consistent with the iteration-9
  // rule that O(V+F) scans must live in effects, not render (iteration 16).
  // `acceleratedRaycast` uses brute force until the tree exists, then auto-upgrades.
  // `indirect: true` keeps the geometry non-indexed so faceIndex == backend face.
  useEffect(() => {
    if (!geometry) return;
    let cancelled = false;
    const id = setTimeout(() => {
      if (cancelled) return;
      try {
        // `indirect: true` is supported at runtime (verified in three-mesh-bvh
        // 0.7.8 build, line 1169/1193) but absent from its .d.ts — cast to satisfy
        // tsc without losing the required "keep geometry non-indexed" behaviour.
        geometry.computeBoundsTree({ indirect: true } as never);
        log.info("MeshDisplay", "BVH ready", { faces: geometry.getAttribute("position")?.count ?? 0 });
      } catch (e) {
        log.warn("MeshDisplay", "computeBoundsTree failed", { error: String(e) });
      }
    }, 0);
    return () => {
      cancelled = true;
      clearTimeout(id);
      geometry.disposeBoundsTree?.();
    };
  }, [geometry]);

  // Precompute adjacency / segment maps ONCE per relevant meshData slice, but
  // DEFERRED to effects so the FIRST paint + surface picking are never blocked
  // by these O(V+F) scans — the source of the multi-second "import lag before
  // I can pick" complaint (iteration 9, problem 2). SegmentOutline /
  // SegmentHighlight and the lasso same-side snap all guard on these being
  // non-null, so a brief null window (until the effect runs, post-paint) is
  // harmless.
  const [edgeMap, setEdgeMap] = useState<{ map: Map<number, number[]>; vmax: number } | null>(null);
  const [vertexData, setVertexData] = useState<{
    vertexFaces: Map<number, number[]>;
    vertexNormals: Float32Array;
  } | null>(null);

  // facesBySeg as synchronous derived value (iteration 23, REFUTE B11). Was
  // useState + useEffect which lagged one commit behind meshData — causing
  // SegmentHighlight to flash off/on when a new region was finalized (the
  // effect hadn't run yet so facesBySeg.get(newLabel) returned undefined).
  // useMemo eliminates that frame delay.
  const facesBySeg = useMemo((): Map<number, number[]> | null => {
    if (!meshData) return null;
    return buildFacesBySegment(meshData);
  }, [meshData?.faces, meshData?.segmentLabels]);

  // edgeMap + vertexData depend only on geometry (faces/vertices), NOT labels —
  // they stay stable across selection / label toggles and only rebuild on a new
  // mesh load.
  useEffect(() => {
    if (!meshData) {
      setEdgeMap(null);
      setVertexData(null);
      return;
    }
    setEdgeMap(buildEdgeMap(meshData));
    setVertexData(buildVertexData(meshData));
  }, [meshData?.faces, meshData?.vertices]);

  // Set of valid segment ids (membership test). Used to decide whether the face
  // under the cursor / clicked for Fill belongs to a real partition. CRITICAL:
  // auto_segment compresses labels to 0..K, so label 0 is a REAL segment — the
  // correct test is membership here, never `segmentLabels[faceId] != 0`.
  const segmentIds = useMemo(
    () => new Set((meshData?.segments ?? []).map((s) => s.id)),
    [meshData?.segments]
  );

  // Singleton empty set to avoid reallocating on every render when there are no
  // segments yet (e.g. between loadModel and autoSegment). Declared BEFORE
  // giantSegmentIds because the latter's factory references it — moving it
  // after would cause a Temporal Dead Zone crash (iteration 24 hotfix).
  const emptySet = useMemo(() => new Set<number>(), []);

  // Giant segments whose hover would paint the entire model teal (iteration 23,
  // REFUTE B1/B2). Phase4 merge_small_regions_fast can roll 10k+ regions into
  // ~27 giants (avg 55k faces each). Hovering any of these floods the overlay —
  // visually identical to "the whole model is highlighted". Fill already guards
  // against this (usePaintTool: realSegs.length > 1 && share <= 0.8); we mirror
  // that guard here so hover and fill agree on what counts as "actionable".
  //
  // A segment is "giant" when:
  //   - it occupies > WHOLE_SEGMENT_MAX_SHARE (80 %) of total faces, OR
  //   - there is only 1 auto segment (everything merged into one blob).
  // Manual regions (label >= MANUAL_SEGMENT_OFFSET) are never silenced — the user
  // explicitly drew them and expects immediate feedback.
  const giantSegmentIds = useMemo((): Set<number> => {
    const segs = meshData?.segments;
    if (!segs || segs.length === 0) return emptySet;
    const total = meshData?.faceCount ?? 0;
    if (total <= 0) return emptySet;
    const autoSegs = segs.filter((s) => (s.id ?? 0) < MANUAL_SEGMENT_OFFSET);
    if (autoSegs.length <= 1) {
      // Single auto-segment case: silence it regardless of share.
      return new Set(autoSegs.map((s) => s.id));
    }
    const threshold = total * WHOLE_SEGMENT_MAX_SHARE;
    return new Set(segs.filter((s) => (s.faceCount ?? 0) > threshold && (s.id ?? 0) < MANUAL_SEGMENT_OFFSET).map((s) => s.id));
  }, [meshData?.segments, meshData?.faceCount]);

  // Raycast to surface, convert world hit → model-local coords.
  // Group is rotated -PI/2 about X, so worldToLocal yields local = (x, -z, y).
  const getLocalHit = useCallback(
    (clientX: number, clientY: number): LocalHit | null => {
      if (!meshRef.current || !geometry) return null;
      const rect = gl.domElement.getBoundingClientRect();
      const mouse = new THREE.Vector2(
        ((clientX - rect.left) / rect.width) * 2 - 1,
        -((clientY - rect.top) / rect.height) * 2 + 1
      );
      raycaster.setFromCamera(mouse, camera);
      const hits = raycaster.intersectObject(meshRef.current, false);
      if (hits.length === 0) return null;
      const h = hits[0];
      // Triangle normal (model-local), used for same-side snapping so lasso
      // points stay on the visible surface instead of leaking to a back face
      // through a thin shell (iteration 7, problem 2).
      let normal = new THREE.Vector3(0, 0, 1);
      const geo = meshRef.current.geometry;
      const idx = geo.index;
      if (h.faceIndex != null) {
        const posAttr = geo.getAttribute("position");
        // Non-indexed geometry (iteration 14): face f's vertices are at
        // 3f, 3f+1, 3f+2. Indexed geometry would need the index indirection.
        const ia = idx ? idx.getX(h.faceIndex * 3) : h.faceIndex * 3;
        const ib = idx ? idx.getX(h.faceIndex * 3 + 1) : h.faceIndex * 3 + 1;
        const ic = idx ? idx.getX(h.faceIndex * 3 + 2) : h.faceIndex * 3 + 2;
        const vA = new THREE.Vector3().fromBufferAttribute(posAttr, ia);
        const vB = new THREE.Vector3().fromBufferAttribute(posAttr, ib);
        const vC = new THREE.Vector3().fromBufferAttribute(posAttr, ic);
        normal = new THREE.Vector3()
          .crossVectors(vB.clone().sub(vA), vC.clone().sub(vA))
          .normalize();
      }
      return {
        point: meshRef.current.worldToLocal(h.point.clone()),
        faceIndex: h.faceIndex ?? -1,
        normal,
      };
    },
    [gl, camera, raycaster, meshRef, geometry]
  );

  // Close the active lasso loop and finalize the manual region. Shared by the
  // distance-based click closure, the Enter key, and any future UI button, so
  // the "clear selection state" logic lives in exactly one place.
  const finalizeLasso = useCallback(async () => {
    const pts = lassoPointsRef.current.map(
      (p) => [p.x, p.y, p.z] as [number, number, number]
    );
    if (pts.length < 3) {
      setStatusMessage("至少需要 3 个点才能闭合选区");
      return;
    }
    const faceIdx = lassoFaceIndicesRef.current.slice();
    lassoPointsRef.current = [];
    lassoFaceIndicesRef.current = [];
    setLassoPoints([]);
    lassoStartPointRef.current = null;
    lassoStartVertexRef.current = null;
    setLassoPreview(null);
    setLassoClosing(false);
    setLassoSnap(null);
    lassoSnapRef.current = null;
    await finalizeManualRegion(pts, faceIdx);
  }, [finalizeManualRegion, setStatusMessage]);

  // Handle a lasso click: snap to vertex, append, or close the loop.
  const handleLassoClick = useCallback(
    async (hit: LocalHit) => {
      const local = hit.point;
      // Pass the hit face so the backend snaps to the SAME-SIDE vertex (front
      // shell), fixing "can't select / curve not preserved" on thin meshes.
      const res = await manualRegionAddPoint([local.x, local.y, local.z], hit.faceIndex);
      if (!res) return;
      const snapped = new THREE.Vector3(res.snapped[0], res.snapped[1], res.snapped[2]);
      const prev = lassoPointsRef.current;
      const startPoint = lassoStartPointRef.current;
      // Robust closure: at least 3 points placed (start + >=2 more) and the new
      // point is within closeThreshold of the START point (distance-based, NOT
      // exact vertex match). The old exact-match test was unreachable on dense
      // meshes, so finalize was never called and the region never appeared.
      // This matches the yellow "closing" preview the user already sees while
      // hovering near the start point.
      if (prev.length >= 2 && startPoint && snapped.distanceTo(startPoint) < closeThreshold) {
        await finalizeLasso();
        return;
      }
      if (prev.length === 0) {
        lassoStartPointRef.current = snapped.clone();
        lassoStartVertexRef.current = res.vertexIndex;
      }
      lassoFaceIndicesRef.current.push(hit.faceIndex);
      const next = [...prev, snapped];
      lassoPointsRef.current = next;
      setLassoPoints(next);
      setStatusMessage(
        `套索：已选 ${next.length} 个点` +
          (next.length >= 2 ? "（点击起点附近闭合，或按 Enter）" : "")
      );
    },
    [manualRegionAddPoint, finalizeLasso, setStatusMessage, closeThreshold]
  );

  const handleFacePicked = useCallback(
    async (faceId: number, opts?: { wholeRegion?: boolean }) => {
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
          setLastPaintDebug(
            `✏️ 分区笔 face=${faceId} → label=${result.segmentLabel}`
          );
        }
      } else {
        const result = await paintFace(faceId, opts);
        if (result) {
          const gpu = updateFaceColors(result.updatedFaces, result.updatedColors);
          const first5 = result.updatedFaces.slice(0, 5).join(",");
          const state = useAppStore.getState();
          setLastPaintDebug(
            `🖌 ${activeTool} face=${faceId} color=${JSON.stringify(state.currentColor)} shade=${state.shadingMode} gpu=${gpu} → ${result.updatedFaces.length} faces [${first5}${result.updatedFaces.length > 5 ? "…" : ""}]`
          );
        }
      }
    },
    [paintFace, paintSegmentFace, updateFaceColors, isSegmentTool, activeTool, setLastPaintDebug]
  );

  const pick = useFacePicker(meshRef, handleFacePicked);

  // Keep a live ref to handleFacePicked so the serial paint queue (below)
  // always calls the latest closure without re-creating the queue.
  const handleFacePickedRef = useRef(handleFacePicked);
  handleFacePickedRef.current = handleFacePicked;

  // Serial paint queue (iteration 15, req #2). A drag fires many pointermove
  // events, each issuing an async Tauri `paintFace`. Fire-and-forget invokes
  // returned OUT OF ORDER and `updateFaceColors` wrote them synchronously, so
  // overlapping brush strokes overwrote each other in the wrong order — the
  // real cause of "color not following the cursor" (REFUTE P6). Serializing
  // guarantees faces are colored strictly in the order the cursor visited them.
  // Latest-wins consumer throttle (iteration 16, REFUTE blocker-3). In WebView2
  // `pointermove` is already rAF-aligned, so an extra rAF-coalesce is a no-op that
  // only trades lag for gaps — instead we keep ONLY the most recent face while a
  // Tauri paint IPC is in flight. The queue never grows unbounded, so the visual
  // paint lags by at most one round-trip. Intermediate faces between two samples
  // are covered by the brush's radius flood-fill (no need for path interpolation
  // in this iteration; see loop-journal backlog for a batch-stroke IPC).
  // The queued item carries the modifier state captured at event time, so a
  // Shift+click whole-region fill is not lost while an earlier paint is in
  // flight (iteration 18, M3).
  const dragPendingFaceRef = useRef<{ faceId: number; wholeRegion: boolean } | null>(null);
  const paintDrainingRef = useRef(false);
  const enqueuePaint = useCallback((faceId: number, wholeRegion = false) => {
    dragPendingFaceRef.current = { faceId, wholeRegion }; // keep freshest cursor face
    if (paintDrainingRef.current) return; // busy → drop intermediate (covered by radius)
    paintDrainingRef.current = true;
    (async () => {
      const fn = handleFacePickedRef.current;
      while (dragPendingFaceRef.current != null) {
        const job = dragPendingFaceRef.current;
        dragPendingFaceRef.current = null;
        try { await fn(job.faceId, { wholeRegion: job.wholeRegion }); }
        catch { /* ignore single-face failures */ }
      }
      paintDrainingRef.current = false;
    })();
  }, []);

  // Alt key → temporary camera rotate even while a paint/segment/lasso tool is
  // active. While held, onPointerDown early-returns (see MeshDisplay) so the
  // LEFT=ROTATE mapping takes over. Release or window-blur resets _altHeld to
  // avoid a stuck "always rotate" state. Ignored when typing in a field.
  useEffect(() => {
    const isEditable = (el: EventTarget | null) =>
      el instanceof HTMLElement &&
      (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.altKey && !e.repeat && !isEditable(e.target)) {
        _altHeld = true;
      }
    };
    const onKeyUp = (e: KeyboardEvent) => {
      if (!e.altKey) _altHeld = false;
    };
    const onBlur = () => { _altHeld = false; };
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
      window.removeEventListener("blur", onBlur);
    };
  }, []);

  // Guarantee camera rotation is enabled whenever the active tool changes. This
  // is now handled by the ControlsBridge mouseButtons remap (iteration 14):
  // paint tools disable LEFT in OrbitControls and map RIGHT→rotate, so no
  // enableRotate toggling is needed here (and toggling it would wrongly block
  // the new RIGHT-drag rotate while painting).

  // Pointer events for painting + hover
  useEffect(() => {
    const canvas = gl.domElement;

    // Single raycast shared by the hover block and the paint-drag branch so we
    // never run a SECOND O(F) pass per pointermove (the hover merge already
    // removed that hotspot; REFUTE: do not reintroduce it in the drag path).
    const raycastFace = (clientX: number, clientY: number): THREE.Intersection | null => {
      const mesh = meshRef.current;
      if (!mesh) return null;
      const rect = canvas.getBoundingClientRect();
      const mouse = new THREE.Vector2(
        ((clientX - rect.left) / rect.width) * 2 - 1,
        -((clientY - rect.top) / rect.height) * 2 + 1
      );
      raycaster.setFromCamera(mouse, camera);
      const hits = raycaster.intersectObject(mesh, false);
      return hits.length > 0 ? hits[0] : null;
    };

    // Drive the brush-cursor ring (BrushCursorImperative reads hoverInfoRef).
    // Only brush tools in paint view show the ring; everything else clears it.
    const updateBrushCursor = (hit: THREE.Intersection | null) => {
      if (!isRadiusTool || segmentView || !meshRef.current || !hit || hit.faceIndex == null) {
        hoverInfoRef.current = null;
        return;
      }
      const pos = meshRef.current.worldToLocal(hit.point.clone());
      const normal = hit.face ? hit.face.normal.clone() : new THREE.Vector3(0, 0, 1);
      hoverInfoRef.current = { position: pos, normal };
    };

    const onPointerDown = (e: PointerEvent) => {
      if (e.button !== 0) return;
      // View mode or Alt held → let OrbitControls handle it (rotate); never paint.
      if (isViewTool || _altHeld) return;
      if (isLassoTool) {
        setHoveredSegment(null);
        const local = getLocalHit(e.clientX, e.clientY);
        // Off-model click: let OrbitControls rotate (iteration 22 fix:
        // lasso previously blocked all LEFT=ROTATE even over empty space).
        if (!local) return;
        handleLassoClick(local);
        return;
      }
      // Paint-like tools (brush/segment/fill/picker): LEFT only paints when the
      // cursor is over the model. Over empty space LEFT stays ROTATE (handled by
      // OrbitControls via the context-sensitive mapping) — so a drag on empty
      // space orbits the camera instead of painting (iteration 15, req #1). We
      // gate isPainting on the initial hit to avoid a camera fight if the drag
      // later crosses onto the model.
      const hit = raycastFace(e.clientX, e.clientY);
      if (!hit || hit.faceIndex == null) {
        // Empty space: do NOT paint, do NOT capture. OrbitControls rotates.
        return;
      }
      // Snapshot current paint state for undo BEFORE the first face is mutated
      // (one stroke = one undo unit).
      const md = useAppStore.getState().meshData;
      if (md && modifiesFaces) {
        pushUndo({
          faceColors: Uint8Array.from(md.faceColors),
          segmentLabels: Uint32Array.from(md.segmentLabels),
        });
      }
      isPainting.current = true;
      // Capture the pointer so pointerup fires on the canvas even when the
      // cursor is released OUTSIDE it.
      try { canvas.setPointerCapture(e.pointerId); } catch { /* not critical */ }
      // Shift + fill click → explicit whole-connected-region flood (the escape
      // hatch that keeps the pre-iteration-18 behaviour reachable, M3).
      enqueuePaint(hit.faceIndex, activeTool === "fill" && e.shiftKey);
    };

    const onPointerMove = (e: PointerEvent) => {
      if (isPainting.current) {
        // Fill and eyedropper tools only act on the initial click, not drag.
        if (activeTool === "fill" || activeTool === "picker") return;
        // One raycast serves BOTH painting and the brush-cursor ring (no second
        // O(F) pass). Paints are serialized via enqueuePaint so overlapping brush
        // strokes apply in cursor order — fixes "color not following the cursor"
        // from out-of-order async Tauri writes (iteration 15, req #2).
        const hit = raycastFace(e.clientX, e.clientY);
        updateBrushCursor(hit && hit.faceIndex != null ? hit : null);
        if (hit && hit.faceIndex != null) {
          enqueuePaint(hit.faceIndex);
        }
        return;
      }

      // Lasso: show rubber-band preview; snap to nearest vertex of hit face.
      if (isLassoTool) {
        const hit = getLocalHit(e.clientX, e.clientY);
        if (!hit) {
          setLassoPreview(null);
          setLassoClosing(false);
          setLassoSnap(null);
          lassoSnapRef.current = null;
          overModelRef.current = false; // iteration 22: refresh so OrbitControls can rotate
          applyCameraButtons();
          return;
        }
        overModelRef.current = true; // iteration 22: refresh so LEFT is correctly disabled
        applyCameraButtons();
        setLassoPreview(hit.point);
        const start = lassoPointsRef.current[0];
        if (start && lassoPointsRef.current.length >= 2) {
          // Closing detection uses the snapped point (iteration 23, REFUTE
          // B18) so the yellow "ready to close" preview agrees with the actual
          // commit behaviour. Previously the preview checked raw hit.point while
          // the commit checked the backend-snapped vertex — on coarse meshes
          // the two could disagree by half a face, causing "shows close but
          // doesn't" or vice versa.
          const checkPoint = lassoSnapRef.current ?? hit.point;
          setLassoClosing(checkPoint.distanceTo(start) < closeThreshold);
        } else {
          setLassoClosing(false);
        }
        // Auto-snap preview: SAME-SIDE nearest vertex to the hit point — the
        // exact mirror of the backend `snap_point_to_vertex_on_face` rule, so
        // the preview dot lands EXACTLY where the committed point lands (no
        // front/back ambiguity on thin shells) and the rubber-band connects
        // predictably. Fixes "I can't pick/connect it" + "curve not preserved".
        const verts = meshData?.vertices;
        if (verts && vertexData) {
          const sv = nearestVertexLocalOnFace(
            verts,
            meshData.faces,
            hit.point,
            hit.faceIndex,
            hit.normal,
            vertexData.vertexFaces,
            vertexData.vertexNormals
          );
          if (sv && (!lassoSnapRef.current || sv.distanceTo(lassoSnapRef.current) > 1e-6)) {
            lassoSnapRef.current = sv.clone();
            setLassoSnap(sv);
          }
        }
        return;
      }

      // Partition hover reporting + brush cursor: share ONE raycast (was 2× O(F)
      // per pointermove — the #1 perf hotspot). Both need hits[0].faceIndex; we
      // raycast once then branch.
      if (!isPainting.current && !isLassoTool && geometry && meshRef.current && meshData) {
        const rect = canvas.getBoundingClientRect();
        const mouse = new THREE.Vector2(
          ((e.clientX - rect.left) / rect.width) * 2 - 1,
          -((e.clientY - rect.top) / rect.height) * 2 + 1
        );
        raycaster.setFromCamera(mouse, camera);
        const hits = raycaster.intersectObject(meshRef.current, false);

        // (0) Track whether the cursor is over the model. Drives the
        // context-sensitive LEFT mapping (paint on model vs rotate on empty,
        // iteration 15 req #1). Reuses the raycast we just did — no extra pass.
        const nowOver = hits.length > 0;
        if (nowOver !== overModelRef.current) {
          overModelRef.current = nowOver;
          applyCameraButtons();
        }

        // (a) Partition hover — silence giant segments (iteration 23, REFUTE
        //     B1/B2). Phase4 merge_small_regions_fast can produce ~27 regions
        //     averaging 55k faces each; highlighting one floods the entire model
        //     teal. Fill already guards this case (share > 0.8 || too few auto
        //     segments); we mirror the guard so hover and fill agree. Manual
        //     regions (label >= MANUAL_SEGMENT_OFFSET) are never silenced — the
        //     user drew them explicitly and expects immediate feedback.
        let nextHover: number | null = null;
        if (hits.length > 0 && hits[0].faceIndex != null) {
          const lbl = meshData.segmentLabels[hits[0].faceIndex];
          if (lbl !== undefined && segmentIds.has(lbl) && !giantSegmentIds.has(lbl)) {
            nextHover = lbl;
          }
        }
        if (useAppStore.getState().hoveredSegment !== nextHover) {
          setHoveredSegment(nextHover);
        }

        // (c) Fill tool face highlight (面片高亮). Mark the exact face under the
        //     cursor with a bright triangle OUTLINE (see FillFaceHighlight) so the
        //     user sees the precise click target. Writes 3 model-local vertices
        //     from the (non-indexed) rendered geometry; cleared on ANY miss so a
        //     stale triangle never lingers when the cursor leaves the model
        //     (iteration 17, REFUTE M1). The partition under the cursor is already
        //     highlighted via hoveredSegment (SegmentHighlight/Outline) above.
        if (activeTool === "fill") {
          if (hits.length > 0 && hits[0].faceIndex != null && meshRef.current) {
            const pa = meshRef.current.geometry.getAttribute("position");
            const fi = hits[0].faceIndex;
            if (!fillHoverFaceRef.current) {
              fillHoverFaceRef.current = [new THREE.Vector3(), new THREE.Vector3(), new THREE.Vector3()];
            }
            const tri = fillHoverFaceRef.current;
            tri[0].set(pa.getX(fi * 3), pa.getY(fi * 3), pa.getZ(fi * 3));
            tri[1].set(pa.getX(fi * 3 + 1), pa.getY(fi * 3 + 1), pa.getZ(fi * 3 + 1));
            tri[2].set(pa.getX(fi * 3 + 2), pa.getY(fi * 3 + 2), pa.getZ(fi * 3 + 2));
          } else {
            fillHoverFaceRef.current = null;
          }
        }

        // (b) Brush cursor — only for brush tools in paint view. Reuses the
        // single raycast above (no second O(F) pass). During a paint drag the
        // drag branch updates hoverInfoRef the same way, so the ring tracks the
        // cursor while drawing (fixes "ring at A, paint at B" disconnect).
        updateBrushCursor(hits.length > 0 ? hits[0] : null);
        return; // ← early return: skip the old separate brush-cursor raycast below
      }

      // Merged raycast path above handles all hover/brush-cursor cases.
      // (The hover-cursor lives in hoverInfoRef, read by BrushCursorImperative
      // via useFrame — no React state to clear here.)
    };

    const onPointerUp = (e: PointerEvent) => {
      try { canvas.releasePointerCapture(e.pointerId); } catch { /* not critical */ }
      if (isPainting.current) {
        isPainting.current = false;
        // Finalize segment after drag ends
        if (activeTool === "segment" && currentSegLabelRef.current !== null) {
          finalizeSegment(currentSegLabelRef.current);
          currentSegLabelRef.current = null;
          segPaintedFacesRef.current.clear();
        }
      }
    };

    const onPointerLeave = () => {
      if (isPainting.current) {
        isPainting.current = false;
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
      hoverInfoRef.current = null;
      fillHoverFaceRef.current = null;
      setHoveredSegment(null);
      overModelRef.current = false; // iteration 22: reset so OrbitControls can rotate
      applyCameraButtons();
    };

    const onPointerCancel = (e: PointerEvent) => {
      try { canvas.releasePointerCapture(e.pointerId); } catch { /* not critical */ }
      isPainting.current = false;
    };

    // Ctrl+Wheel over the model resizes the brush (iteration 15, req #3). Intercept
    // on the canvas' PARENT with capture:true so this runs in the capture phase
    // BEFORE OrbitControls' bubble-phase wheel listener on the canvas, letting us
    // stopPropagation + preventDefault to suppress zoom. (Attaching capture on
    // the canvas itself does NOT preempt a same-element listener registered
    // earlier — confirmed against three-stdlib OrbitControls, REFUTE P3.)
    const onWheel = (e: WheelEvent) => {
      if (e.ctrlKey && isRadiusTool && overModelRef.current) {
        e.preventDefault();
        e.stopPropagation();
        const cur = useAppStore.getState().brushRadius;
        const next = Math.min(200, Math.max(0.5, cur - e.deltaY * 0.02));
        useAppStore.getState().setBrushRadius(next);
        setStatusMessage(`笔刷半径 ${next.toFixed(1)}mm（Ctrl+滚轮）`);
      }
    };
    const wheelTarget = canvas.parentElement ?? canvas;
    wheelTarget.addEventListener("wheel", onWheel, { capture: true });

    canvas.addEventListener("pointerdown", onPointerDown);
    canvas.addEventListener("pointermove", onPointerMove);
    canvas.addEventListener("pointerup", onPointerUp);
    canvas.addEventListener("pointerleave", onPointerLeave);
    canvas.addEventListener("pointercancel", onPointerCancel);

    return () => {
      wheelTarget.removeEventListener("wheel", onWheel, { capture: true });
      canvas.removeEventListener("pointerdown", onPointerDown);
      canvas.removeEventListener("pointermove", onPointerMove);
      canvas.removeEventListener("pointerup", onPointerUp);
      canvas.removeEventListener("pointerleave", onPointerLeave);
      canvas.removeEventListener("pointercancel", onPointerCancel);
    };
  }, [gl.domElement, pick, handleFacePicked, enqueuePaint, activeTool, isBrushTool, isRadiusTool, isSegmentTool, isLassoTool, segmentView, geometry, raycaster, camera, getLocalHit, handleLassoClick, closeThreshold, meshData, vertexData, segmentIds, setHoveredSegment]);

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
        lassoFaceIndicesRef.current = [];
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
        lassoFaceIndicesRef.current = lassoFaceIndicesRef.current.slice(0, -1);
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
      if (e.key === "Enter") {
        e.preventDefault();
        if (lassoPointsRef.current.length >= 3) {
          finalizeLasso();
        } else {
          setStatusMessage("至少需要 3 个点才能闭合选区");
        }
        return;
      }
      // Ctrl/Cmd+Z / Ctrl/Cmd+Y / Ctrl/Cmd+Shift+Z are handled by the global
      // undo/redo effect below so paint-stroke undo/redo works outside lasso.
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [isLassoTool, setStatusMessage, t, manualRegionUndo, finalizeLasso]);

  // Global undo/redo — covers paint strokes AND lasso regions, and works in any
  // tool (undo of a paint stroke while View tool is active, etc.):
  //   Ctrl/Cmd+Z       → undo (lasso point if a loop is open → last lasso region
  //                      → last paint/segment stroke)
  //   Ctrl/Cmd+Y       → redo paint stroke
  //   Ctrl/Cmd+Shift+Z → redo paint stroke
  // Ignored while typing in a text field so normal editing is unaffected.
  useEffect(() => {
    const isEditable = (el: EventTarget | null) =>
      el instanceof HTMLElement &&
      (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);
    const onKey = (e: KeyboardEvent) => {
      if (isEditable(e.target)) return;
      const mod = e.ctrlKey || e.metaKey;
      if (!mod) return;
      const isRedo =
        e.key === "y" || e.key === "Y" ||
        ((e.key === "z" || e.key === "Z") && e.shiftKey);
      const isUndo = (e.key === "z" || e.key === "Z") && !e.shiftKey;
      if (!isUndo && !isRedo) return;
      e.preventDefault();

      const tool = useAppStore.getState().activeTool;
      const lassoActive = tool === "lasso";
      const hasLoop = lassoPointsRef.current.length > 0;

      if (isRedo) {
        if (lassoActive && hasLoop) return; // no per-point redo in lasso
        redoPaint();
        return;
      }

      // Undo
      if (lassoActive) {
        if (hasLoop) {
          const next = lassoPointsRef.current.slice(0, -1);
          lassoPointsRef.current = next;
          lassoFaceIndicesRef.current = lassoFaceIndicesRef.current.slice(0, -1);
          setLassoPoints(next);
          if (next.length === 0) lassoStartVertexRef.current = null;
          setLassoClosing(false);
          setStatusMessage(
            t("lasso.undoPoint") + (next.length > 0 ? `（剩 ${next.length} 个点）` : "")
          );
        } else {
          manualRegionUndo();
        }
      } else {
        undoPaint();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [redoPaint, undoPaint, manualRegionUndo, setStatusMessage, t]);

  if (!geometry) return null;

  return (
    <group rotation={[-Math.PI / 2, 0, 0]}>
      <mesh ref={meshRef} geometry={geometry} castShadow receiveShadow>
        {/* Shading mode toggle (iteration 19). "flat" = meshBasicMaterial:
            exact per-face color, no lighting — ideal for final colour
            verification but unshaded geometry looks like a silhouette when
            unpainted. "shaded" = meshLambertMaterial: Lambert diffuse gives
            enough 3D shape readability without PBR specular complexity;
            slight angle-dependent brightness remains. */}
        {shadingMode === "flat" ? (
          <meshBasicMaterial vertexColors side={THREE.DoubleSide} />
        ) : (
          <meshLambertMaterial vertexColors side={THREE.DoubleSide} />
        )}
      </mesh>
      {showWireframe && (
        <lineSegments>
          <wireframeGeometry args={[geometry]} />
          <lineBasicMaterial color={0x333333} opacity={0.05} transparent />
        </lineSegments>
      )}
      {/* Brush radius preview cursor — imperative, useFrame-driven */}
      {isRadiusTool && !segmentView && (
        <BrushCursorImperative hoverInfoRef={hoverInfoRef} brushRadius={brushRadius} color={currentColor} />
      )}
      {/* Fill tool: highlight the exact face under the cursor (面片高亮).
          Drawn as an outline so it marks the click target without implying the
          full fill scope (iteration 17). */}
      {activeTool === "fill" && (
        <FillFaceHighlight faceRef={fillHoverFaceRef} />
      )}
      {/* Paint view: highlight ONLY the segment under the cursor (near
          highlight). The just-created partition does NOT stay highlighted here —
          it lights up when you hover near it, which is the requested behavior. */}
      {!segmentView && hoveredSegment !== null && meshData && edgeMap && facesBySeg && (
        <>
          <SegmentHighlight meshData={meshData} selectedSegment={hoveredSegment} facesBySeg={facesBySeg} />
          <SegmentOutline meshData={meshData} selectedSegment={hoveredSegment} edgeMap={edgeMap} facesBySeg={facesBySeg} />
        </>
      )}
      {/* Segment view: always highlight the selected segment (fill + outline);
          when the cursor nears a DIFFERENT partition, show that partition's
          outline so the user can see what they are about to operate on. */}
      {segmentView && meshData && edgeMap && facesBySeg && (
        <>
          {selectedSegment !== null && (
            <>
              <SegmentHighlight meshData={meshData} selectedSegment={selectedSegment} facesBySeg={facesBySeg} />
              <SegmentOutline meshData={meshData} selectedSegment={selectedSegment} edgeMap={edgeMap} facesBySeg={facesBySeg} />
            </>
          )}
          {hoveredSegment !== null && hoveredSegment !== selectedSegment && (
            <SegmentOutline meshData={meshData} selectedSegment={hoveredSegment} edgeMap={edgeMap} facesBySeg={facesBySeg} />
          )}
        </>
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
    background: "var(--overlay-bg, rgba(0,0,0,0.6))",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    zIndex: 10,
  },
  card: {
    background: "var(--bg-root, #1e1e1e)",
    borderRadius: 12,
    padding: "24px 32px",
    textAlign: "center",
    minWidth: 280,
    boxShadow: "0 4px 24px var(--shadow, rgba(0,0,0,0.3))",
  },
  spinner: {
    fontSize: 36,
    marginBottom: 12,
  },
  stageText: {
    color: "var(--text-1, #cccccc)",
    fontSize: 14,
    marginBottom: 12,
    minHeight: 20,
  },
  barOuter: {
    width: "100%",
    height: 8,
    background: "var(--border-strong, #333333)",
    borderRadius: 4,
    overflow: "hidden",
  },
  barInner: {
    height: "100%",
    background: "var(--gradient, linear-gradient(90deg, #4a9eff, #00d4ff))",
    borderRadius: 4,
    transition: "width 0.3s ease",
  },
  pctText: {
    color: "var(--accent-text, #4a9eff)",
    fontSize: 20,
    fontWeight: 700,
    marginTop: 8,
  },
};

// ─── In-app Debug HUD ────────────────────────────────────────────
/// Shows the last paint/pick operation detail as a small monospace line. This
/// replaces the F12 console (unavailable in Tauri release builds) so the user
/// can verify, without devtools, that painting lands on the face under the
/// cursor (iteration 14).
function DebugHud() {
  const debug = useAppStore((s) => s.lastPaintDebug);
  if (!debug) return null;
  return <div style={debugStyles.box}>{debug}</div>;
}

const debugStyles: Record<string, React.CSSProperties> = {
  box: {
    position: "absolute",
    bottom: 32,
    left: 12,
    background: "var(--debug-bg, rgba(20,24,28,0.85))",
    color: "var(--success, #9fe7a0)",
    border: "1px solid var(--success-border, #2e7d32)",
    borderRadius: 6,
    padding: "4px 8px",
    fontSize: 11,
    fontFamily: "ui-monospace, Menlo, Consolas, monospace",
    zIndex: 6,
    pointerEvents: "none",
    maxWidth: 520,
    whiteSpace: "pre-wrap",
    wordBreak: "break-all",
  },
};

// ─── Controls Help Bar ────────────────────────────────────────────
function ControlsHelp() {
  const t = useT();
  const activeTool = useAppStore((s) => s.activeTool);
  const isBrush =
    activeTool === "brush" || activeTool === "spray" ||
    activeTool === "smart" || activeTool === "eraser";
  // Context-sensitive mapping (iteration 15, req #1/#3): over the model LEFT
  // paints, over empty space LEFT rotates, RIGHT always pans, Ctrl+Wheel
  // resizes the brush (brush tools only).
  const hint =
    activeTool === "view"
      ? "左键 旋转 · 右键 平移 · 中键 缩放 · 滚轮 缩放"
      : isBrush
      ? "模型上 左键绘制 · 空白处左键旋转 · 右键 平移 · Ctrl+滚轮 调笔刷"
      : "左键 绘制/选取 · 右键 平移 · 中键 缩放 · 滚轮 缩放";
  return (
    <div style={helpStyles.bar}>
      <span style={helpStyles.item}>{hint}</span>
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
    background: "var(--help-bg, rgba(0,0,0,0.5))",
    color: "var(--text-2, #aaaaaa)",
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
  sep: { color: "var(--border, #555555)", margin: "0 4px" },
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
        className="cym-btn"
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
    border: "1px solid var(--border, #555555)",
    borderRadius: 6,
    background: "var(--bg-panel, #2d2d2d)",
    color: "var(--text-1, #cccccc)",
    cursor: "pointer",
    fontSize: 13,
  },
  buttonActive: {
    borderColor: "var(--accent, #4a9eff)",
    background: "var(--bg-active, #3a5a7a)",
    color: "var(--accent-text, #4a9eff)",
  },
};

// ─── Transient Toast ──────────────────────────────────────────────
/// Shows a short-lived popup (e.g. "添加分区成功") and auto-dismisses.
function Toast() {
  const toast = useAppStore((s) => s.toast);
  const setToast = useAppStore((s) => s.setToast);
  useEffect(() => {
    if (!toast) return;
    const id = setTimeout(() => setToast(null), 2500);
    return () => clearTimeout(id);
  }, [toast, setToast]);
  if (!toast) return null;
  return (
    <div style={toastStyles.box}>
      <span style={toastStyles.icon}>✅</span>
      <span style={toastStyles.text}>{toast}</span>
    </div>
  );
}

const toastStyles: Record<string, React.CSSProperties> = {
  box: {
    position: "absolute",
    top: 56,
    left: "50%",
    transform: "translateX(-50%)",
    background: "var(--toast-bg, rgba(34,40,48,0.95))",
    color: "var(--toast-text, #e8f0ff)",
    border: "1px solid var(--accent, #4a9eff)",
    borderRadius: 8,
    padding: "10px 18px",
    fontSize: 14,
    display: "flex",
    alignItems: "center",
    gap: 8,
    zIndex: 20,
    boxShadow: "0 6px 24px var(--shadow, rgba(0,0,0,0.3))",
    pointerEvents: "none",
  },
  icon: { fontSize: 16 },
  text: { fontWeight: 600 },
};

// ─── Viewport Root ────────────────────────────────────────────────
export function Viewport() {
  const t = useT();
  const isLoaded = useAppStore((s) => s.isLoaded);

  return (
    <div style={{ width: "100%", height: "100%", position: "relative" }}>
      <Canvas
        orthographic
        camera={{ position: [50, 50, 50], zoom: 1 }}
        gl={{ antialias: true, preserveDrawingBuffer: true }}
        style={{ background: "var(--bg-canvas, #2a2a2a)" }}
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
      <DebugHud />
      <Toast />
      {!isLoaded && (
        <div
          style={{
            position: "absolute",
            top: "50%",
            left: "50%",
            transform: "translate(-50%, -50%)",
            color: "var(--text-3, #888888)",
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
