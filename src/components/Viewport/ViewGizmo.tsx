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
  isAxisEdgeVisible,
  isPointerInGizmo,
  setGizmoRect,
  snapRotation,
} from "./viewGizmoModel";

// ─── Orca-style view gizmo ────────────────────────────────────────
// Bottom-right corner widget: a labelled cube (Orca-style direction words:
// 顶部/正面/... or TOP/FRONT/...) plus model-space X/Y/Z axis rods welded to
// one cube corner (ImGuizmo ViewManipulate port). It mirrors the main camera
// orientation, can be DRAGGED anywhere inside its guard circle to orbit the
// camera, and clicking a face / axis letter snaps (animated) to the matching
// axis-aligned view.
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
// Corner axis triad, ported 1:1 from ImGuizmo's ViewManipulate (the widget
// OrcaSlicer uses for its 3D navigator): the three model axes are WELDED to
// one fixed corner of the cube — corner = (-h,-h,+h), the front-lower-left
// vertex in the default view — each running exactly along its cube edge
// (origin → adjacent vertex, one edge long) with the letter just past the
// end. Lines are depth-test-free overlays drawn on top; an axis whose edge
// lies on the far side is dimmed to 35% instead of being occluded.
const TRIAD_CORNER = new THREE.Vector3(-CUBE / 2, -CUBE / 2, CUBE / 2);
const AXIS_LINE_LEN = CUBE; // one cube edge
const AXIS_LABEL_AT = 1.02; // from the corner, just past the adjacent vertex
const AXIS_DIM_ALPHA = 0.6;
const AXIS_R = 0.035; // rod radius (~2px on screen) — lines alone read too faint
// Outer guard ring: a thin FIXED screen-space circle framing the whole
// widget. Hidden by default; fades in as soon as the pointer enters the
// circle — the whole disc inside is the drag surface.
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
  },
  light: {
    cube: "#e9eaec",
    edge: "#b7bcc4",
    faceBg: "#f2f3f5",
    faceBorder: "#b7bcc4",
    faceText: "#2f3237",
    hover: "#2f7de1",
  },
} as const;

// ─── Canvas textures (face direction words / axis letters) ─────────

interface GizmoPalette {
  cube: string;
  edge: string;
  faceBg: string;
  faceBorder: string;
  faceText: string;
  hover: string;
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

function makeHeadTexture(label: string, color: string): THREE.CanvasTexture {
  // Orca-style bare letter at the line end (no circle backing).
  const canvas = document.createElement("canvas");
  canvas.width = 64;
  canvas.height = 64;
  const ctx = canvas.getContext("2d")!;
  ctx.font = 'bold 38px "Segoe UI", Arial, sans-serif';
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillStyle = color;
  ctx.fillText(label, 32, 34);
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
const _ray = new THREE.Raycaster();
const _ndc = new THREE.Vector2();
const AXIS_DIRS = GIZMO_AXES.map((a) => new THREE.Vector3(...a.worldDir));

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
  // The same listener feeds the "pointer inside the grab circle" flag that
  // drives the ring's fade-in (the circle is the affordance, not just the
  // cube) — plain DOM math instead of R3F pointer state, which is per
  // event-layer inside the Hud portal.
  const pointerInsideCircle = useRef(false);
  useEffect(() => {
    const parent = gl.domElement.parentElement;
    if (!parent) return;
    const ringPx = (RING_R + 0.05) * S;
    const sync = (e: PointerEvent | WheelEvent) => {
      if (!controls) return;
      // Gesture-owner stability: once a button is down, the gesture keeps its
      // initial owner — flipping controls.enabled mid-orbit (cursor sweeping
      // across the corner) visibly freezes the rotation for a few frames.
      if (e.type === "pointermove" && e.buttons !== 0) return;
      // One rect read per event, shared by the square guard and the circle test.
      const r = gl.domElement.getBoundingClientRect();
      const inside = isPointerInGizmo(e.clientX, e.clientY, gl.domElement, r);
      controls.enabled = !inside;
      if (e.target === gl.domElement) {
        pointerInsideCircle.current =
          Math.hypot(e.clientX - (r.left + r.width - GIZMO_MARGIN_X), e.clientY - (r.top + r.height - GIZMO_MARGIN_Y)) <= ringPx;
      }
    };
    const onLeave = () => {
      pointerInsideCircle.current = false;
    };
    parent.addEventListener("pointerdown", sync, true);
    parent.addEventListener("pointermove", sync, true);
    parent.addEventListener("wheel", sync, true);
    gl.domElement.addEventListener("pointerleave", onLeave);
    return () => {
      parent.removeEventListener("pointerdown", sync, true);
      parent.removeEventListener("pointermove", sync, true);
      parent.removeEventListener("wheel", sync, true);
      gl.domElement.removeEventListener("pointerleave", onLeave);
      if (controls) controls.enabled = true;
    };
  }, [gl, controls]);

  return (
    <GizmoHelper alignment="bottom-right" margin={[GIZMO_MARGIN_X, GIZMO_MARGIN_Y]}>
      <GizmoContent
        mainCamera={mainCamera}
        controls={controls}
        invalidate={invalidate}
        pointerInsideCircle={pointerInsideCircle}
      />
    </GizmoHelper>
  );
}

// ─── Gizmo content (inside the HUD portal) ─────────────────────────

function GizmoContent({
  mainCamera,
  controls,
  invalidate,
  pointerInsideCircle,
}: {
  mainCamera: THREE.Camera;
  controls: ControlsLike | null;
  invalidate: () => void;
  pointerInsideCircle: React.MutableRefObject<boolean>;
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
    () => GIZMO_AXES.map((a) => makeHeadTexture(a.label, a.color)),
    []
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

  // Axis overlay rods + letter sprite materials (ImGuizmo ViewManipulate
  // port — see TRIAD_CORNER comment above). Rods are depth-test-free so they
  // draw over the cube exactly like ImGuizmo's 2D draw-list lines, but thick
  // enough to read clearly.
  const axisArt = useMemo(() => {
    const rods = GIZMO_AXES.map((a, i) => {
      const geo = new THREE.CylinderGeometry(AXIS_R, AXIS_R, AXIS_LINE_LEN, 10);
      const mat = new THREE.MeshBasicMaterial({
        color: a.color,
        transparent: true,
        depthTest: false,
        toneMapped: false,
      });
      const rod = new THREE.Mesh(geo, mat);
      // Cylinder default axis is +Y — orient onto the model axis and centre
      // the rod on the edge it represents.
      rod.quaternion.setFromUnitVectors(new THREE.Vector3(0, 1, 0), AXIS_DIRS[i]);
      rod.position.copy(TRIAD_CORNER).addScaledVector(AXIS_DIRS[i], AXIS_LINE_LEN / 2);
      rod.renderOrder = 998;
      return rod;
    });
    const spriteMats = GIZMO_AXES.map(
      () => new THREE.SpriteMaterial({ transparent: true, depthTest: false })
    );
    return { rods, spriteMats };
  }, []);
  useEffect(() => {
    axisArt.spriteMats.forEach((m, i) => {
      m.map = headTextures[i];
      m.needsUpdate = true;
    });
  }, [axisArt, headTextures]);
  useEffect(
    () => () => {
      axisArt.rods.forEach((rod) => {
        rod.geometry.dispose();
        (rod.material as THREE.Material).dispose();
      });
      axisArt.spriteMats.forEach((m) => m.dispose());
    },
    [axisArt]
  );

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
      // Pure-model rotation (camera −Z-looks-at-target convention). An
      // Object3D.lookAt here pointed +Z at the target and snapped every view
      // to the antipode — click 顶部, land at the bottom (B3, GUI audit
      // 2026-10-02; the animation's `to·from⁻¹` also carries the position
      // offset, so the wrong quaternion flips BOTH heading and landing site).
      animRef.current = {
        from: mainCamera.quaternion.clone(),
        to: snapRotation(view.dir, view.up),
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
  const discRef = useRef<THREE.Mesh>(null);
  useFrame((_, delta) => {
    // HUD camera is fixed and axis-aligned (it renders the corner overlay),
    // so "screen-aligned" == world-identity. The gizmo group carries camQ⁻¹,
    // hence the ring compensation: localQ = camQ gives worldQ = I (a fixed
    // screen-plane circle). The invisible grab disc gets the same treatment.
    if (ringRef.current && discRef.current) {
      ringRef.current.quaternion.copy(mainCamera.quaternion);
      discRef.current.quaternion.copy(mainCamera.quaternion);
      // The circle IS the affordance: show it as soon as the pointer enters
      // the grab disc (not only over the cube) — everything inside is
      // grabbable. Flag maintained by ViewGizmo's DOM capture listener.
      const target = pointerInsideCircle.current ? RING_OPACITY : 0;
      ringOpacityRef.current += (target - ringOpacityRef.current) * Math.min(1, delta * 14);
      ringRef.current.visible = ringOpacityRef.current > 0.01;
      if (ringMatRef.current) ringMatRef.current.opacity = ringOpacityRef.current;
    }
    // ImGuizmo far-side dimming (see isAxisEdgeVisible): dim an axis whose
    // cube edge lies on the far side instead of letting it occlude.
    for (let i = 0; i < GIZMO_AXES.length; i++) {
      const visible = isAxisEdgeVisible(
        TRIAD_CORNER,
        AXIS_DIRS,
        i,
        mainCamera.quaternion,
        CUBE / 2
      );
      const alpha = visible ? 1 : AXIS_DIM_ALPHA;
      (axisArt.rods[i].material as THREE.MeshBasicMaterial).opacity = alpha;
      const view = GIZMO_AXES[i].posView;
      axisArt.spriteMats[i].opacity = alpha;
      axisArt.spriteMats[i].color.set(hover?.view === view ? pal.hover : "#ffffff");
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
          frame — fades in while the pointer is inside the grab circle) */}
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

      {/* Invisible grab disc: the whole circle is the drag surface — a
          pointerdown anywhere inside reaches the group drag handler through
          bubbling; fully transparent but raycastable. Screen-aligned per
          frame (see useFrame), same as the ring. */}
      <mesh ref={discRef} renderOrder={997}>
        <circleGeometry args={[RING_R, 64]} />
        <meshBasicMaterial transparent opacity={0} depthWrite={false} depthTest={false} />
      </mesh>

      {/* Axis triad welded to the cube's fixed corner (ImGuizmo port): each
          axis runs exactly along its cube edge as a depth-free overlay rod,
          with the bare letter just past the adjacent vertex; far-side axes
          dim to 60% (see useFrame). Letter sprites stay clickable for the
          axis-view snap. */}
      {GIZMO_AXES.map((axis, i) => {
        const view = axis.posView;
        return (
          <group key={axis.label}>
            <primitive object={axisArt.rods[i]} />
            <sprite
              ref={addPickable}
              position={TRIAD_CORNER.clone().addScaledVector(AXIS_DIRS[i], AXIS_LABEL_AT)}
              scale={0.26}
              material={axisArt.spriteMats[i]}
              userData={{ viewIndex: view }}
              onPointerMove={(e) => {
                e.stopPropagation();
                if (hover?.view !== view) setHover({ view, onCube: false });
              }}
              onPointerOut={(e) => {
                e.stopPropagation();
                setHover((h) => (h?.view === view ? null : h));
              }}
            />
          </group>
        );
      })}
    </group>
  );
}
