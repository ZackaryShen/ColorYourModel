import * as THREE from "three";

// ─── View gizmo shared model (Orca-style axis + view cube) ─────────
// Pure data + a tiny screen-rect registry shared between the R3F gizmo
// (ViewGizmo.tsx, rendered through drei's Hud portal) and the DOM-level
// pointer handlers in Viewport.tsx (paint / lasso / brush wheel /
// OrbitControls), which must ignore pointers that land on the gizmo corner.
//
// Axis conventions: everything below is expressed in THREE WORLD axes. The
// model group is rotated -PI/2 about X (see CameraFit), so the mapping from
// MODEL axes (what a 3D-print user cares about, Z = print height) is:
//   model X → world +X    model Y → world -Z    model Z → world +Y
// All view/face/axis tables below encode that mapping once, here.

/** GizmoHelper alignment is "bottom-right"; these margins (CSS px from the
 *  canvas edges to the gizmo CENTRE) must mirror the values passed to
 *  <GizmoHelper margin={...}> in ViewGizmo.tsx so the DOM-side exclusion
 *  rect stays in lock-step with the rendered gizmo position. */
export const GIZMO_MARGIN_X = 72;
export const GIZMO_MARGIN_Y = 96;
/** Half extent of the interactive corner square (guard ring + slack).
 *  Generous on purpose: a click that misses the gizmo visuals but lands in
 *  the corner must not paint or orbit — it hits dead space instead. */
export const GIZMO_HALF_EXTENT = 86;

export interface GizmoRect {
  /** Canvas-local centre (CSS px, top-left origin). */
  x: number;
  y: number;
  /** Half extent; 0 disables the rect (no gizmo mounted). */
  half: number;
}

let gizmoRect: GizmoRect = { x: 0, y: 0, half: 0 };

/** Gizmo centre for a canvas of the given CSS size — mirrors drei
 *  GizmoHelper's bottom-right + margin placement math. */
export function gizmoCenterFor(canvasWidth: number, canvasHeight: number): { x: number; y: number } {
  return { x: canvasWidth - GIZMO_MARGIN_X, y: canvasHeight - GIZMO_MARGIN_Y };
}

/** Called by ViewGizmo whenever the canvas size changes. */
export function setGizmoRect(canvasWidth: number, canvasHeight: number): void {
  const c = gizmoCenterFor(canvasWidth, canvasHeight);
  gizmoRect = { x: c.x, y: c.y, half: GIZMO_HALF_EXTENT };
}

/** Called on gizmo unmount — re-opens the corner to normal input. */
export function clearGizmoRect(): void {
  gizmoRect = { x: 0, y: 0, half: 0 };
}

/** Pure hit-test core (unit-testable, no DOM). Canvas-local coords. */
export function isPointInGizmoRect(lx: number, ly: number, rect: GizmoRect): boolean {
  if (rect.half <= 0) return false;
  return Math.abs(lx - rect.x) <= rect.half && Math.abs(ly - rect.y) <= rect.half;
}

/** Client coords → canvas-local → gizmo hit test. Pass a precomputed canvas
 *  rect to avoid a duplicate getBoundingClientRect when the caller already
 *  read one (forced-layout reads per pointermove are jank fuel). */
export function isPointerInGizmo(
  clientX: number,
  clientY: number,
  canvas: HTMLCanvasElement,
  rect?: DOMRect
): boolean {
  const r = rect ?? canvas.getBoundingClientRect();
  if (r.width === 0 || r.height === 0) return false;
  return isPointInGizmoRect(clientX - r.left, clientY - r.top, gizmoRect);
}

// ─── Canonical views ──────────────────────────────────────────────

export type GizmoPlane = "XY" | "XZ" | "YZ";

export interface GizmoView {
  /** i18n key for the status message shown after snapping. */
  key: string;
  /** i18n key for the short direction word baked into the cube face
   *  (Orca-style: 顶部/正面/... in zh, TOP/FRONT/... in en). */
  faceKey: string;
  /** Where the CAMERA sits relative to the orbit target (world axes). */
  dir: [number, number, number];
  /** Screen-up hint: fixes the roll of the snapped view and keeps lookAt
   *  well-defined at the poles (top/bottom). Camera.up itself is untouched
   *  so OrbitControls keeps its Y-up turntable semantics. */
  up: [number, number, number];
  /** Model plane the user faces in this view (status-bar message only —
   *  the cube faces are labelled with direction words). */
  plane: GizmoPlane;
}

/** The six axis-aligned views, in GIZMO_FACES/GIZMO_AXES viewIndex order:
 *  0 top, 1 bottom, 2 front, 3 back, 4 right, 5 left. */
export const GIZMO_VIEWS: GizmoView[] = [
  { key: "gizmo.top", faceKey: "gizmo.face.top", dir: [0, 1, 0], up: [0, 0, -1], plane: "XY" },
  { key: "gizmo.bottom", faceKey: "gizmo.face.bottom", dir: [0, -1, 0], up: [0, 0, -1], plane: "XY" },
  { key: "gizmo.front", faceKey: "gizmo.face.front", dir: [0, 0, -1], up: [0, 1, 0], plane: "XZ" },
  { key: "gizmo.back", faceKey: "gizmo.face.back", dir: [0, 0, 1], up: [0, 1, 0], plane: "XZ" },
  { key: "gizmo.right", faceKey: "gizmo.face.right", dir: [1, 0, 0], up: [0, 1, 0], plane: "YZ" },
  { key: "gizmo.left", faceKey: "gizmo.face.left", dir: [-1, 0, 0], up: [0, 1, 0], plane: "YZ" },
];

/** One labelled face of the cube. */
export interface GizmoFaceDef {
  /** World-space outward normal (axis-aligned unit vector). */
  normal: [number, number, number];
  /** Texture-up direction on the cube surface (dice-style lettering so the
   *  label reads upright from its natural viewing side). */
  faceUp: [number, number, number];
  plane: GizmoPlane;
  viewIndex: number;
}

export const GIZMO_FACES: GizmoFaceDef[] = [
  { normal: [0, 1, 0], faceUp: [0, 0, -1], plane: "XY", viewIndex: 0 }, // top
  { normal: [0, -1, 0], faceUp: [0, 0, -1], plane: "XY", viewIndex: 1 }, // bottom
  { normal: [0, 0, -1], faceUp: [0, 1, 0], plane: "XZ", viewIndex: 2 }, // front
  { normal: [0, 0, 1], faceUp: [0, 1, 0], plane: "XZ", viewIndex: 3 }, // back
  { normal: [1, 0, 0], faceUp: [0, 1, 0], plane: "YZ", viewIndex: 4 }, // right
  { normal: [-1, 0, 0], faceUp: [0, 1, 0], plane: "YZ", viewIndex: 5 }, // left
];

/** Axis arrows labelled in MODEL space. The arrow direction is where the
 *  model axis points in world coords after the -PI/2 X group rotation. */
export interface GizmoAxisDef {
  label: "X" | "Y" | "Z";
  color: string;
  worldDir: [number, number, number];
  /** View index the positive / negative arrow head snaps to. */
  posView: number;
  negView: number;
}

export const GIZMO_AXES: GizmoAxisDef[] = [
  { label: "X", color: "#ff5a5a", worldDir: [1, 0, 0], posView: 4, negView: 5 },
  { label: "Y", color: "#4ade80", worldDir: [0, 0, -1], posView: 2, negView: 3 },
  { label: "Z", color: "#60a5fa", worldDir: [0, 1, 0], posView: 0, negView: 1 },
];

/**
 * Camera orientation for a canonical gizmo view, as a pure quaternion.
 *
 * Builds the rotation a REAL camera needs: local −Z looks at the target from
 * `dir`, local +Y matches `up`. Constructed via `Matrix4.lookAt(eye, target,
 * up)` — whose `_z = normalize(eye − target)` lands local +Z on `dir` — so the
 * caller never instantiates a throwaway camera. This matters: an Object3D's
 * `lookAt` points +Z at the target (three r170 `three.module.js:7516`), and
 * feeding that quaternion to the main camera snapped every gizmo view to the
 * ANTIPODE (click 顶部 → camera lands at the bottom looking up; GUI audit
 * 2026-10-02, B3).
 *
 * `Matrix4.lookAt` also carries the degenerate up‖dir fallback for free, so
 * no hand-rolled cross products here.
 */
export function snapRotation(dir: [number, number, number], up: [number, number, number]): THREE.Quaternion {
  const eye = new THREE.Vector3(...dir);
  const target = new THREE.Vector3(0, 0, 0);
  const m = new THREE.Matrix4().lookAt(eye, target, new THREE.Vector3(...up));
  return new THREE.Quaternion().setFromRotationMatrix(m);
}

/** Orientation for a face-label plane: local +Y = faceUp, local +Z = the
 *  outward normal (plane fronts face the viewer). */
export function facePlaneQuaternion(face: GizmoFaceDef): THREE.Quaternion {
  const n = new THREE.Vector3(...face.normal);
  const u = new THREE.Vector3(...face.faceUp);
  const right = new THREE.Vector3().crossVectors(u, n).normalize();
  const m = new THREE.Matrix4().makeBasis(right, u, n);
  return new THREE.Quaternion().setFromRotationMatrix(m);
}

// Scratch for isAxisEdgeVisible — module scope, reused per call (the gizmo
// calls this 3x per frame; no per-frame allocation).
const _visInv = new THREE.Quaternion();
const _visProbe = new THREE.Vector3();

/** ImGuizmo ViewManipulate far-side test, as a pure function: is the cube
 *  edge starting at `corner` and running along `dirs[axisIndex]` on the near
 *  side? Probes the edge's two adjacent face centres (edge midpoint ±
 *  halfExtent along the other two axes); if either faces the viewer the axis
 *  is fully lit, otherwise the caller dims it. `camQ` is the MAIN camera's
 *  world quaternion — the HUD overlay camera is axis-aligned (looking down
 *  -Z), so a probe's world z > 0 means it faces the viewer. */
export function isAxisEdgeVisible(
  corner: THREE.Vector3,
  dirs: readonly THREE.Vector3[],
  axisIndex: number,
  camQ: THREE.Quaternion,
  halfExtent: number
): boolean {
  _visInv.copy(camQ).invert();
  for (let j = 1; j <= 2; j++) {
    _visProbe.copy(corner)
      .addScaledVector(dirs[axisIndex], halfExtent)
      .addScaledVector(dirs[(axisIndex + j) % 3], halfExtent)
      .applyQuaternion(_visInv);
    if (_visProbe.z > 0) return true;
  }
  return false;
}
