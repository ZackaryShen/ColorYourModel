import { describe, it, expect } from "vitest";
import * as THREE from "three";
import {
  GIZMO_AXES,
  GIZMO_FACES,
  GIZMO_HALF_EXTENT,
  GIZMO_MARGIN_X,
  GIZMO_MARGIN_Y,
  GIZMO_VIEWS,
  facePlaneQuaternion,
  gizmoCenterFor,
  isAxisEdgeVisible,
  isPointInGizmoRect,
} from "./viewGizmoModel";

const len = (v: [number, number, number]) => Math.hypot(v[0], v[1], v[2]);
const dot = (
  a: [number, number, number],
  b: [number, number, number]
) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];

// The view tables encode the app's Z-up printing convention once: the model
// group is rotated -PI/2 about X, so model Y → world -Z and model Z (print
// height) → world +Y. If this mapping drifts, the gizmo labels lie about the
// model's axes and "click the top face → XY plane" breaks.
describe("viewGizmoModel", () => {
  it("defines the six axis-aligned views with unique unit dirs and ups ⊥ dirs", () => {
    expect(GIZMO_VIEWS).toHaveLength(6);
    const dirs = new Set<string>();
    const keys = new Set<string>();
    const faceKeys = new Set<string>();
    for (const v of GIZMO_VIEWS) {
      expect(len(v.dir)).toBeCloseTo(1, 6);
      expect(len(v.up)).toBeCloseTo(1, 6);
      // Screen-up must not be parallel to the view direction, or lookAt is
      // degenerate (the whole point of the up hint).
      expect(Math.abs(dot(v.dir, v.up))).toBeLessThan(1e-6);
      dirs.add(v.dir.join(","));
      keys.add(v.key);
      // Every view carries a distinct cube-face label key (Orca-style
      // direction words baked into the face textures).
      expect(v.faceKey).toMatch(/^gizmo\.face\./);
      faceKeys.add(v.faceKey);
    }
    expect(dirs.size).toBe(6);
    expect(keys.size).toBe(6);
    expect(faceKeys.size).toBe(6);
  });

  it("labels each view with the plane perpendicular to its view direction", () => {
    // Camera along ±Y looks at the world XZ plane = model XY plane, etc.
    for (const v of GIZMO_VIEWS) {
      const [x, y, z] = v.dir;
      const expected = Math.abs(y) > 0.5 ? "XY" : Math.abs(z) > 0.5 ? "XZ" : "YZ";
      expect(v.plane).toBe(expected);
    }
  });

  it("covers all six cube faces and matches each face to its view", () => {
    expect(GIZMO_FACES).toHaveLength(6);
    const normals = new Set<string>();
    for (const f of GIZMO_FACES) {
      expect(len(f.normal)).toBeCloseTo(1, 6);
      normals.add(f.normal.join(","));
      const view = GIZMO_VIEWS[f.viewIndex];
      expect(view).toBeDefined();
      // Facing the face's outward normal = camera sits along that normal.
      expect(view.dir.join(",")).toBe(f.normal.join(","));
      expect(view.plane).toBe(f.plane);
    }
    expect(normals.size).toBe(6);
  });

  it("maps model axes to world dirs with Z-up semantics (Z→+Y, Y→−Z)", () => {
    const byLabel = Object.fromEntries(GIZMO_AXES.map((a) => [a.label, a]));
    expect(byLabel.X.worldDir).toEqual([1, 0, 0]);
    expect(byLabel.Y.worldDir).toEqual([0, 0, -1]);
    expect(byLabel.Z.worldDir).toEqual([0, 1, 0]);
  });

  it("snaps arrow heads to the views along their own direction", () => {
    expect(GIZMO_AXES).toHaveLength(3);
    for (const a of GIZMO_AXES) {
      expect(len(a.worldDir)).toBeCloseTo(1, 6);
      const pos = GIZMO_VIEWS[a.posView];
      const neg = GIZMO_VIEWS[a.negView];
      expect(pos.dir.join(",")).toBe(a.worldDir.join(","));
      expect(neg.dir.join(",")).toBe(a.worldDir.map((c) => -c).join(","));
    }
  });

  it("builds right-handed, orthonormal face-label orientations", () => {
    for (const face of GIZMO_FACES) {
      const q = facePlaneQuaternion(face);
      const m = new THREE.Matrix4().makeRotationFromQuaternion(q);
      const ex = new THREE.Vector3().setFromMatrixColumn(m, 0);
      const ey = new THREE.Vector3().setFromMatrixColumn(m, 1);
      const ez = new THREE.Vector3().setFromMatrixColumn(m, 2);
      // Local +Z must be the outward normal; local +Y the face-up hint.
      const closeTo = (v: THREE.Vector3, e: [number, number, number]) => {
        expect(v.x).toBeCloseTo(e[0], 6);
        expect(v.y).toBeCloseTo(e[1], 6);
        expect(v.z).toBeCloseTo(e[2], 6);
      };
      closeTo(ez, face.normal);
      closeTo(ey, face.faceUp);
      // Right-handed basis: X = Y × Z keeps plane text unmirrored.
      expect(ex.crossVectors(ey, ez).length()).toBeCloseTo(1, 6);
      expect(m.determinant()).toBeCloseTo(1, 6);
    }
  });

  it("places the gizmo centre at canvas size minus the bottom-right margins", () => {
    expect(gizmoCenterFor(800, 600)).toEqual({
      x: 800 - GIZMO_MARGIN_X,
      y: 600 - GIZMO_MARGIN_Y,
    });
  });

  it("hit-tests the exclusion rect inclusively and disables on half=0", () => {
    const rect = { x: 700, y: 500, half: GIZMO_HALF_EXTENT };
    expect(isPointInGizmoRect(700, 500, rect)).toBe(true); // centre
    expect(isPointInGizmoRect(700 - GIZMO_HALF_EXTENT, 500, rect)).toBe(true); // edge
    expect(isPointInGizmoRect(700 + GIZMO_HALF_EXTENT + 1, 500, rect)).toBe(false);
    expect(isPointInGizmoRect(700, 500, { x: 700, y: 500, half: 0 })).toBe(false);
  });

  // Far-side dimming probe (ImGuizmo ViewManipulate semantics). The HUD
  // overlay camera is axis-aligned looking down -Z, so with the identity
  // main-camera rotation the corner (-h,-h,+h) is the near vertex: the X and
  // Z edges are lit, the Y edge (model Y = world -Z) runs away from the
  // viewer and must dim.
  it("dims exactly the away-pointing axis edge at the default view", () => {
    const corner = new THREE.Vector3(-0.4, -0.4, 0.4);
    const dirs = GIZMO_AXES.map((a) => new THREE.Vector3(...a.worldDir));
    const identity = new THREE.Quaternion();
    expect(isAxisEdgeVisible(corner, dirs, 0, identity, 0.4)).toBe(true); // X
    expect(isAxisEdgeVisible(corner, dirs, 1, identity, 0.4)).toBe(false); // Y (away)
    expect(isAxisEdgeVisible(corner, dirs, 2, identity, 0.4)).toBe(true); // Z
  });

  it("dims every edge when the camera turns 180 degrees about world Y", () => {
    const corner = new THREE.Vector3(-0.4, -0.4, 0.4);
    const dirs = GIZMO_AXES.map((a) => new THREE.Vector3(...a.worldDir));
    const flipped = new THREE.Quaternion().setFromAxisAngle(
      new THREE.Vector3(0, 1, 0),
      Math.PI
    );
    // rotY(180) sends the toward-viewer axis from +Z to -Z, so the welded
    // (-h,-h,+h) corner becomes the fully far vertex — all three edges dim.
    expect(isAxisEdgeVisible(corner, dirs, 0, flipped, 0.4)).toBe(false);
    expect(isAxisEdgeVisible(corner, dirs, 1, flipped, 0.4)).toBe(false);
    expect(isAxisEdgeVisible(corner, dirs, 2, flipped, 0.4)).toBe(false);
  });
});
