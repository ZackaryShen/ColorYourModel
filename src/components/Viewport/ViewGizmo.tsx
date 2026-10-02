import * as THREE from "three";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useFrame, useThree, type ThreeEvent } from "@react-three/fiber";
import { GizmoHelper } from "@react-three/drei";
import { useAppStore } from "../../store/appStore";
import { useT } from "../../i18n";
import { translate } from "../../i18nDict";
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
// Corner axis triad: anchored at the cube's bottom-right, sized to match the
// cube. The anchor lives in the counter-rotated frame, so it stays put on
// screen while the arrows sweep/foreshorten with the camera.
const TRIAD_ORIGIN: [number, number, number] = [0.62, -0.62, 0];
const TRIAD_SHAFT_LEN = 0.52;
const TRIAD_HEAD_AT = 0.6;
// Outer guard ring: a thin screen-aligned circle framing the whole widget.
// Hidden by default; fades in only while the pointer hovers the CUBE.
// Must clear the triad's worst-case reach (TRIAD_ORIGIN + head + half sprite).
const RING_R = 1.44;
const RING_W = 0.02;
const RING_OPACITY = 0.35;
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
  // Shrink long direction words (BOTTOM/FRONT) so they stay inside the face.
  const px = label.length >= 5 ? 30 : 42;
  ctx.font = `bold ${px}px "Segoe UI", Arial, sans-serif`;
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
  const language = useAppStore((s) => s.language);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  // Hover target: which view it highlights, and whether it lives on the cube
  // (drives the guard-ring fade-in).
  const [hover, setHover] = useState<{ view: number; onCube: boolean } | null>(null);

  const pal = THEME[theme];
  // Orca-style direction words (顶部/正面/... or TOP/FRONT/...), re-baked on
  // language and theme changes.
  const faceTextures = useMemo(
    () =>
      GIZMO_FACES.map((f) =>
        makeFaceTexture(translate(GIZMO_VIEWS[f.viewIndex].faceKey, language), pal)
      ),
    [pal, language]
  );
  const headTextures = useMemo(
    () => GIZMO_AXES.map((a) => makeHeadTexture(a.label, a.color, pal)),
    [pal]
  );
  useEffect(() => {
    return () => {
      faceTextures.forEach((tex) => tex.dispose());
      headTextures.forEach((tex) => tex.dispose());
    };
  }, [faceTextures, headTextures]);

  const cubeEdges = useMemo(
    () => new THREE.EdgesGeometry(new THREE.BoxGeometry(CUBE, CUBE, CUBE)),
    []
  );
  useEffect(() => () => cubeEdges.dispose(), [cubeEdges]);

  // Click-pick targets: the 6 face planes + the 3 triad arrow heads.
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
  const ringRef = useRef<THREE.Mesh>(null);
  const ringMatRef = useRef<THREE.MeshBasicMaterial>(null);
  const ringOpacityRef = useRef(0);
  useFrame((_, delta) => {
    // Guard ring stays screen-aligned while the cube counter-rotates: the
    // gizmo group carries camQ⁻¹, so a child needs localQ = camQ² for its
    // world orientation to equal the camera's (a billboard facing the viewer).
    if (ringRef.current) {
      ringRef.current.quaternion.copy(mainCamera.quaternion).multiply(mainCamera.quaternion);
      // Fade in only while the pointer is over the cube; a fixed perfect
      // circle that never rotates with the gizmo.
      const target = hover?.onCube ? RING_OPACITY : 0;
      ringOpacityRef.current += (target - ringOpacityRef.current) * Math.min(1, delta * 14);
      ringRef.current.visible = ringOpacityRef.current > 0.01;
      if (ringMatRef.current) ringMatRef.current.opacity = ringOpacityRef.current;
    }
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

      {/* Outer guard ring (thin, screen-aligned circle; opacity driven per
          frame — fades in while the pointer hovers the cube) */}
      <mesh ref={ringRef} visible={false}>
        <ringGeometry args={[RING_R, RING_R + RING_W, 96]} />
        <meshBasicMaterial
          ref={ringMatRef}
          color={pal.faceText}
          opacity={0}
          transparent
          side={THREE.DoubleSide}
          depthWrite={false}
          toneMapped={false}
        />
      </mesh>

      {/* Labelled faces (Orca-style direction words) */}
      {GIZMO_FACES.map((f, i) => (
        <mesh
          key={`face-${i}`}
          ref={addPickable}
          position={new THREE.Vector3(...f.normal).multiplyScalar(CUBE / 2 + 0.006)}
          quaternion={faceQuats[i]}
          userData={{ viewIndex: f.viewIndex }}
          onPointerMove={(e) => {
            e.stopPropagation();
            if (hover?.view !== f.viewIndex) setHover({ view: f.viewIndex, onCube: true });
          }}
          onPointerOut={(e) => {
            e.stopPropagation();
            setHover((h) => (h?.view === f.viewIndex ? null : h));
          }}
        >
          <planeGeometry args={[0.72, 0.72]} />
          <meshBasicMaterial
            map={faceTextures[i] ?? null}
            color={hover?.view === f.viewIndex ? pal.hover : "#ffffff"}
            toneMapped={false}
          />
        </mesh>
      ))}

      {/* Corner axis triad (model space: X, Y=world -Z, Z=world +Y), anchored
          at the cube's bottom-right. The anchor is fixed in the
          counter-rotated frame, so it stays bottom-right on screen while the
          arrows foreshorten with the camera. */}
      <group position={TRIAD_ORIGIN}>
        <mesh>
          <sphereGeometry args={[0.07, 16, 12]} />
          <meshBasicMaterial color={pal.faceText} opacity={0.55} transparent toneMapped={false} />
        </mesh>
        {GIZMO_AXES.map((axis, i) => {
          const dir = new THREE.Vector3(...axis.worldDir);
          const shaftQuat = new THREE.Quaternion().setFromUnitVectors(new THREE.Vector3(0, 1, 0), dir);
          const view = axis.posView;
          return (
            <group key={axis.label}>
              <mesh
                position={dir.clone().multiplyScalar(TRIAD_SHAFT_LEN / 2)}
                quaternion={shaftQuat}
              >
                <cylinderGeometry args={[SHAFT_R, SHAFT_R, TRIAD_SHAFT_LEN, 12]} />
                <meshBasicMaterial color={axis.color} toneMapped={false} />
              </mesh>
              <sprite
                ref={addPickable}
                position={dir.clone().multiplyScalar(TRIAD_HEAD_AT)}
                scale={0.3}
                userData={{ viewIndex: view }}
                onPointerMove={(e) => {
                  e.stopPropagation();
                  if (hover?.view !== view) setHover({ view, onCube: false });
                }}
                onPointerOut={(e) => {
                  e.stopPropagation();
                  setHover((h) => (h?.view === view ? null : h));
                }}
              >
                <spriteMaterial
                  map={headTextures[i]}
                  color={hover?.view === view ? pal.hover : "#ffffff"}
                  toneMapped={false}
                />
              </sprite>
            </group>
          );
        })}
      </group>
    </group>
  );
}
