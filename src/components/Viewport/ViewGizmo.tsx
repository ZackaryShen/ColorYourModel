import * as THREE from "three";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useFrame, useThree, type ThreeEvent } from "@react-three/fiber";
import { GizmoHelper } from "@react-three/drei";
import { useAppStore } from "../../store/appStore";
import { useT } from "../../i18n";
import {
  GIZMO_AXES,
  GIZMO_FACES,
  GIZMO_MARGIN_X,
  GIZMO_MARGIN_Y,
  GIZMO_VIEWS,
  clearGizmoRect,
  facePlaneQuaternion,
  isPointerInGizmo,
  setGizmoRect,
} from "./viewGizmoModel";

// ─── Orca-style view gizmo ────────────────────────────────────────
// Bottom-right corner widget: a labelled cube (plane codes XY/XZ/YZ) plus
// model-space X/Y/Z arrows. It mirrors the main camera orientation, can be
// DRAGGED to orbit the camera, and clicking a face / arrow head snaps
// (animated) to the matching axis-aligned view.
//
// Structure: <ViewGizmo> runs in the MAIN R3F tree — it owns everything that
// must see the main camera/controls (drag orbit, snap animation, the
// OrbitControls enable-guard). Its child <GizmoContent> is passed through
// drei's <GizmoHelper>, which portals it into a corner viewport with its own
// ortho camera and event layer; inside that portal useThree(camera) is the
// HUD camera, so GizmoContent only renders visuals + raycasts picks, and
// receives mainCamera/controls as props.
//
// Input isolation: the app's paint/lasso/wheel handlers are DOM-level canvas
// listeners and OrbitControls listens on the canvas too — neither knows about
// R3F event layers. Two guards keep the corner exclusive to the gizmo:
//   1. Viewport.tsx handlers early-return via isPointerInGizmo() (rect set
//      by this component from the canvas size).
//   2. A capture-phase listener on the canvas' PARENT (ancestor capture is
//      guaranteed to run before canvas listeners, unlike same-element
//      capture — same trick as the Ctrl+wheel brush resizer) toggles
//      controls.enabled off while the pointer is inside the rect.

/** Minimal OrbitControls surface used here (avoids importing three-stdlib types). */
interface ControlsLike {
  target: THREE.Vector3;
  update(): void;
  enabled: boolean;
}

// Gizmo geometry, in unscaled units (the content group scales by S; the HUD
// ortho camera maps 1 unit = 1 CSS px, so S is the pixel size of the cube).
const S = 58; // group scale
const CUBE = 0.8; // cube edge (46px) — half-extent 0.4
const SHAFT_R = 0.032;
const POS_SHAFT_LEN = 0.86; // 0.42 → 1.28
const POS_HEAD_AT = 1.4;
const NEG_SHAFT_LEN = 0.62; // 0.42 → 1.04
const NEG_HEAD_AT = 1.14;
const DRAG_THRESHOLD_PX = 4;
const ROT_SPEED = 0.008; // rad per CSS px of drag
const SNAP_DURATION = 0.28; // seconds

const THEME = {
  dark: {
    cube: "#34383f",
    edge: "#22252a",
    faceBg: "#3d424b",
    faceBorder: "#22252a",
    faceText: "#e8eaed",
    hover: "#7ab0ff",
    headText: "#ffffff",
  },
  light: {
    cube: "#e9eaec",
    edge: "#b7bcc4",
    faceBg: "#f2f3f5",
    faceBorder: "#b7bcc4",
    faceText: "#2f3237",
    hover: "#2f7de1",
    headText: "#ffffff",
  },
} as const;

// ─── Canvas textures (face plane codes / axis letters) ─────────────

interface GizmoPalette {
  cube: string;
  edge: string;
  faceBg: string;
  faceBorder: string;
  faceText: string;
  hover: string;
  headText: string;
}

function makeFaceTexture(label: string, pal: GizmoPalette): THREE.CanvasTexture {
  const canvas = document.createElement("canvas");
  canvas.width = 128;
  canvas.height = 128;
  const ctx = canvas.getContext("2d")!;
  ctx.fillStyle = pal.faceBg;
  ctx.fillRect(0, 0, 128, 128);
  ctx.strokeStyle = pal.faceBorder;
  ctx.lineWidth = 4;
  ctx.strokeRect(0, 0, 128, 128);
  ctx.font = 'bold 40px "Segoe UI", Arial, sans-serif';
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillStyle = pal.faceText;
  ctx.fillText(label, 64, 66);
  const tex = new THREE.CanvasTexture(canvas);
  tex.anisotropy = 4;
  return tex;
}

function makeHeadTexture(label: string, color: string, pal: GizmoPalette): THREE.CanvasTexture {
  const canvas = document.createElement("canvas");
  canvas.width = 64;
  canvas.height = 64;
  const ctx = canvas.getContext("2d")!;
  ctx.beginPath();
  ctx.arc(32, 32, 26, 0, Math.PI * 2);
  ctx.closePath();
  ctx.fillStyle = color;
  ctx.fill();
  if (label) {
    ctx.font = 'bold 30px "Segoe UI", Arial, sans-serif';
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillStyle = pal.headText;
    ctx.fillText(label, 32, 34);
  }
  const tex = new THREE.CanvasTexture(canvas);
  tex.anisotropy = 4;
  return tex;
}

// ─── Snap animation state ─────────────────────────────────────────

interface SnapAnim {
  from: THREE.Quaternion;
  to: THREE.Quaternion;
  /** Camera offset from target at animation start (length preserved). */
  offset: THREE.Vector3;
  target: THREE.Vector3;
  t: number;
}

// Scratch objects — module scope, reused every frame (no per-frame allocs).
const _sph = new THREE.Spherical();
const _offset = new THREE.Vector3();
const _q = new THREE.Quaternion();
const _dq = new THREE.Quaternion();
const _dummy = new THREE.Object3D();
const _ray = new THREE.Raycaster();
const _ndc = new THREE.Vector2();

export function ViewGizmo() {
  const gl = useThree((s) => s.gl);
  const size = useThree((s) => s.size);
  const invalidate = useThree((s) => s.invalidate);
  const mainCamera = useThree((s) => s.camera);
  const controls = useThree((s) => s.controls) as ControlsLike | null;

  // Keep the DOM-side exclusion rect in lock-step with the rendered gizmo.
  useEffect(() => {
    setGizmoRect(size.width, size.height);
    invalidate(); // first paint may land in an idle "demand" window
    return () => clearGizmoRect();
  }, [size.width, size.height, invalidate]);

  // OrbitControls gate: disable the controls whenever the pointer is inside
  // the gizmo corner, so a gizmo press cannot also start an orbit. Runs in
  // the CAPTURE phase on the canvas' PARENT — ancestor capture always fires
  // before the canvas listeners OrbitControls registers, regardless of
  // registration order (same pattern as the Ctrl+wheel brush resizer).
  useEffect(() => {
    const parent = gl.domElement.parentElement;
    if (!parent) return;
    const sync = (e: PointerEvent | WheelEvent) => {
      if (!controls) return;
      controls.enabled = !isPointerInGizmo(e.clientX, e.clientY, gl.domElement);
    };
    parent.addEventListener("pointerdown", sync, true);
    parent.addEventListener("pointermove", sync, true);
    parent.addEventListener("wheel", sync, true);
    return () => {
      parent.removeEventListener("pointerdown", sync, true);
      parent.removeEventListener("pointermove", sync, true);
      parent.removeEventListener("wheel", sync, true);
      if (controls) controls.enabled = true;
    };
  }, [gl, controls]);

  return (
    <GizmoHelper alignment="bottom-right" margin={[GIZMO_MARGIN_X, GIZMO_MARGIN_Y]}>
      <GizmoContent mainCamera={mainCamera} controls={controls} invalidate={invalidate} />
    </GizmoHelper>
  );
}

// ─── Gizmo content (inside the HUD portal) ─────────────────────────

function GizmoContent({
  mainCamera,
  controls,
  invalidate,
}: {
  mainCamera: THREE.Camera;
  controls: ControlsLike | null;
  invalidate: () => void;
}) {
  const t = useT();
  const gl = useThree((s) => s.gl);
  // Inside GizmoHelper's Hud portal this is the HUD ortho camera — the one
  // the click pick below must raycast with.
  const hudCamera = useThree((s) => s.camera);
  const theme = useAppStore((s) => s.theme);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const [hoverView, setHoverView] = useState<number | null>(null);

  const pal = THEME[theme];
  const faceTextures = useMemo(() => {
    const m = new Map<string, THREE.CanvasTexture>();
    for (const p of ["XY", "XZ", "YZ"] as const) m.set(p, makeFaceTexture(p, pal));
    return m;
  }, [pal]);
  const headTextures = useMemo(
    () => GIZMO_AXES.map((a) => makeHeadTexture(a.label, a.color, pal)),
    [pal]
  );
  // Unlettered circles for the negative arrow heads (a sprite without a map
  // renders as a square, which reads as debris next to the round heads).
  const negHeadTextures = useMemo(
    () => GIZMO_AXES.map((a) => makeHeadTexture("", a.color, pal)),
    [pal]
  );
  useEffect(() => {
    return () => {
      faceTextures.forEach((tex) => tex.dispose());
      headTextures.forEach((tex) => tex.dispose());
      negHeadTextures.forEach((tex) => tex.dispose());
    };
  }, [faceTextures, headTextures, negHeadTextures]);

  const cubeEdges = useMemo(
    () => new THREE.EdgesGeometry(new THREE.BoxGeometry(CUBE, CUBE, CUBE)),
    []
  );
  useEffect(() => () => cubeEdges.dispose(), [cubeEdges]);

  // Click-pick targets: the 6 face planes + the 6 arrow heads.
  const pickables = useRef<(THREE.Mesh | THREE.Sprite)[]>([]);
  const addPickable = useCallback(
    (el: THREE.Mesh | THREE.Sprite | null) => {
      if (el) pickables.current.push(el);
    },
    []
  );
  useEffect(() => {
    return () => {
      pickables.current = [];
    };
  }, []);

  // ── camera ops ──
  const orbitBy = useCallback(
    (dxPx: number, dyPx: number) => {
      if (!controls) return;
      _offset.copy(mainCamera.position).sub(controls.target);
      _sph.setFromVector3(_offset);
      _sph.theta -= dxPx * ROT_SPEED;
      _sph.phi -= dyPx * ROT_SPEED;
      _sph.phi = Math.max(0.02, Math.min(Math.PI - 0.02, _sph.phi));
      _sph.makeSafe();
      _offset.setFromSpherical(_sph);
      mainCamera.position.copy(controls.target).add(_offset);
      mainCamera.lookAt(controls.target);
      // Skipped: controls.update() — it would re-lookAt with the default
      // Y up and fight the roll of a just-snapped pole view. OrbitControls
      // re-derives its spherical from the live camera position on the next
      // drag, so the shared state stays coherent without it.
      invalidate();
    },
    [controls, mainCamera, invalidate]
  );

  const animRef = useRef<SnapAnim | null>(null);

  const snapTo = useCallback(
    (viewIndex: number) => {
      const view = GIZMO_VIEWS[viewIndex];
      if (!view || !controls) return;
      _dummy.position
        .copy(controls.target)
        .addScaledVector(new THREE.Vector3(...view.dir), mainCamera.position.distanceTo(controls.target));
      _dummy.up.set(...view.up);
      _dummy.lookAt(controls.target);
      animRef.current = {
        from: mainCamera.quaternion.clone(),
        to: _dummy.quaternion.clone(),
        offset: mainCamera.position.clone().sub(controls.target),
        target: controls.target.clone(),
        t: 0,
      };
      setStatusMessage(t(view.key));
      invalidate();
    },
    [controls, mainCamera, setStatusMessage, t]
  );
  useFrame((_, delta) => {
    const a = animRef.current;
    if (!a) return;
    a.t = Math.min(1, a.t + delta / SNAP_DURATION);
    const e = a.t * a.t * (3 - 2 * a.t); // smoothstep
    _q.copy(a.from).slerp(a.to, e);
    _dq.copy(a.from).invert().premultiply(_q); // _dq = _q * from⁻¹
    mainCamera.quaternion.copy(_q);
    mainCamera.position.copy(a.offset).applyQuaternion(_dq).add(a.target);
    invalidate();
    if (a.t >= 1) animRef.current = null;
  });

  // ── drag / click machinery ──
  // R3F pointer capture routes moves/ups to the group while dragging; a
  // release under DRAG_THRESHOLD_PX counts as a click and is resolved by a
  // manual raycast (R3F's synthetic click would target the group, not the
  // face/head under the cursor, once capture is active).
  const drag = useRef<{ id: number; x: number; y: number; moved: boolean } | null>(null);

  const onGroupDown = (e: ThreeEvent<PointerEvent>) => {
    e.stopPropagation();
    animRef.current = null; // a new gesture interrupts a running snap
    drag.current = { id: e.pointerId, x: e.clientX, y: e.clientY, moved: false };
    (e.target as Element).setPointerCapture(e.pointerId);
    invalidate();
  };

  const onGroupMove = (e: ThreeEvent<PointerEvent>) => {
    const d = drag.current;
    if (!d || e.pointerId !== d.id) return;
    e.stopPropagation();
    const dx = e.clientX - d.x;
    const dy = e.clientY - d.y;
    if (!d.moved && Math.hypot(dx, dy) < DRAG_THRESHOLD_PX) return;
    d.moved = true;
    d.x = e.clientX;
    d.y = e.clientY;
    orbitBy(dx, dy);
  };

  const onGroupUp = (e: ThreeEvent<PointerEvent>) => {
    const d = drag.current;
    if (!d || e.pointerId !== d.id) return;
    e.stopPropagation();
    drag.current = null;
    try {
      (e.target as Element).releasePointerCapture(e.pointerId);
    } catch {
      /* already released */
    }
    if (d.moved) return;
    // Click: pick the face/head under the cursor with the HUD camera.
    const rect = gl.domElement.getBoundingClientRect();
    _ndc.set(
      ((e.clientX - rect.left) / rect.width) * 2 - 1,
      -((e.clientY - rect.top) / rect.height) * 2 + 1
    );
    _ray.setFromCamera(_ndc, hudCamera);
    const hits = _ray.intersectObjects(pickables.current, false);
    if (hits.length > 0) {
      const vi = hits[0].object.userData.viewIndex;
      if (typeof vi === "number") snapTo(vi);
    }
  };

  const faceQuats = useMemo(() => GIZMO_FACES.map(facePlaneQuaternion), []);

  return (
    <group
      scale={S}
      onPointerDown={onGroupDown}
      onPointerMove={onGroupMove}
      onPointerUp={onGroupUp}
    >
      {/* Cube body */}
      <mesh>
        <boxGeometry args={[CUBE, CUBE, CUBE]} />
        <meshBasicMaterial color={pal.cube} toneMapped={false} />
      </mesh>
      <lineSegments geometry={cubeEdges} scale={1.001}>
        <lineBasicMaterial color={pal.edge} toneMapped={false} />
      </lineSegments>

      {/* Labelled faces */}
      {GIZMO_FACES.map((f, i) => (
        <mesh
          key={`face-${i}`}
          ref={addPickable}
          position={new THREE.Vector3(...f.normal).multiplyScalar(CUBE / 2 + 0.006)}
          quaternion={faceQuats[i]}
          userData={{ viewIndex: f.viewIndex }}
          onPointerMove={(e) => {
            e.stopPropagation();
            if (hoverView !== f.viewIndex) setHoverView(f.viewIndex);
          }}
          onPointerOut={(e) => {
            e.stopPropagation();
            setHoverView((h) => (h === f.viewIndex ? null : h));
          }}
        >
          <planeGeometry args={[0.72, 0.72]} />
          <meshBasicMaterial
            map={faceTextures.get(f.plane) ?? null}
            color={hoverView === f.viewIndex ? pal.hover : "#ffffff"}
            toneMapped={false}
          />
        </mesh>
      ))}

      {/* Axis arrows (model space: X, Y=world -Z, Z=world +Y) */}
      {GIZMO_AXES.map((axis, i) => {
        const dir = new THREE.Vector3(...axis.worldDir);
        const shaftQuat = new THREE.Quaternion().setFromUnitVectors(new THREE.Vector3(0, 1, 0), dir);
        const mkShaft = (len: number) => (
          <mesh
            position={dir.clone().multiplyScalar(0.4 + len / 2)}
            quaternion={shaftQuat}
          >
            <cylinderGeometry args={[SHAFT_R, SHAFT_R, len, 12]} />
            <meshBasicMaterial color={axis.color} toneMapped={false} />
          </mesh>
        );
        const mkHead = (at: number, view: number, labelled: boolean) => (
          <sprite
            ref={addPickable}
            position={dir.clone().multiplyScalar(at)}
            scale={labelled ? 0.34 : 0.22}
            userData={{ viewIndex: view }}
            onPointerMove={(e) => {
              e.stopPropagation();
              if (hoverView !== view) setHoverView(view);
            }}
            onPointerOut={(e) => {
              e.stopPropagation();
              setHoverView((h) => (h === view ? null : h));
            }}
          >
            <spriteMaterial
              map={labelled ? headTextures[i] : negHeadTextures[i]}
              color={labelled ? (hoverView === view ? pal.hover : "#ffffff") : "#ffffff"}
              opacity={labelled ? 1 : 0.55}
              transparent
              toneMapped={false}
            />
          </sprite>
        );
        return (
          <group key={axis.label}>
            {mkShaft(POS_SHAFT_LEN)}
            {mkShaft(NEG_SHAFT_LEN)}
            {mkHead(POS_HEAD_AT, axis.posView, true)}
            {mkHead(NEG_HEAD_AT, axis.negView, false)}
          </group>
        );
      })}
    </group>
  );
}
