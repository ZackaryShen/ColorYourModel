/**
 * core.mjs — the forensic model of ColorYourModel's seed-erase click.
 *
 * PURE JavaScript. No three.js, no DOM. Used by BOTH:
 *   - app.mjs   (browser: the 3D scene + HUD, which cross-checks every value
 *                here against the real three.js camera / Raycaster / worldToLocal)
 *   - probe.mjs (headless: prints the same table to stdout)
 *
 * Every formula below is transcribed from the real app, with the source line
 * numbers of the branch this app was built against (HEAD 64d544e,
 * feature_012_governance):
 *
 *   src/components/Viewport/Viewport.tsx
 *     :2580   <group rotation={[-Math.PI / 2, 0, 0]}>   ← the Z-up → Y-up group
 *     :1059-1066  seedMarkerSize = max(0.0054, diag * 0.0016)
 *     :1076-1111  eraseNearWorldPoint  (threshold = seedMarkerSize * 2.5)
 *     :1425-1462  getLocalHit          (raycast in world, then worldToLocal)
 *     :1791-1802  raycastFace          (returns the WORLD-space hit point)
 *     :1939-1990  eraseAtScreen        (R6/R7 screen-space attempt, 32 px)
 *     :1991-1992  R8-final click path  → eraseNearWorldPoint(local.point)
 *     :2125-2126  R8-final smear path  → eraseNearWorldPoint(world hit point)
 *     :165-217    CameraFit            (orthographic fit formula)
 *     :2704-2760  SEED_GEOMETRY / SeedMarker (the "sapling" glyph)
 *     :2786-2792  billboard: g.quaternion.copy(cam.quaternion) on a CHILD of
 *                 the rotated group  ← the billboard bug
 *     :3441-3451  <Canvas orthographic camera={{position:[50,50,50], zoom:1}}>
 *
 * KEY FACT the whole exhibit is built on:
 *   • Seeds are stored MODEL-LOCAL, as the centroid of the clicked triangle,
 *     computed from meshData.vertices / meshData.faces (STL layout, Z-up).
 *     (Viewport.tsx:2065-2078, src/types/mesh.ts:42-49)
 *   • The mesh AND the seed markers live inside a group rotated -PI/2 about X,
 *     so  world = R_x(-90°) · local  =  (x, z, -y);  local = (X, -Z, Y).
 *   • Any strategy that feeds model-local numbers to a world-space API (or vice
 *     versa) is comparing two different coordinate systems. That is the bug.
 */

// ─── constants, transcribed ──────────────────────────────────────────────────

/** Viewport.tsx:1065 — Math.max(0.0054, diag * 0.0016) */
export const SEED_MARKER_SIZE_MIN = 0.0054;
export const SEED_MARKER_SIZE_DIAG = 0.0016;
/** Viewport.tsx:1090 — const thresh = seedMarkerSize * 2.5; */
export const ERASE_THRESHOLD_FACTOR = 2.5;
/** Viewport.tsx:1968 — if (best >= 0 && bestD <= 32)  ← screen-space R6/R7 test */
export const SCREEN_THRESHOLD_PX = 32;
/** Viewport.tsx:193 — const dir = new THREE.Vector3(0.4, 0.4, 1).normalize(); */
const FIT_DIR_RAW = [0.4, 0.4, 1];
/** Viewport.tsx:2580 — rotation={[-Math.PI / 2, 0, 0]} */
export const GROUP_ROT_X = -Math.PI / 2;
/** Probe's canonical viewport (CSS px). The browser reports its real size. */
export const CANONICAL_VIEWPORT = { width: 1280, height: 800 };

// ─── tiny vector helpers (plain [x,y,z] triples) ─────────────────────────────

export const add3 = (a, b) => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
export const sub3 = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
export const scale3 = (a, s) => [a[0] * s, a[1] * s, a[2] * s];
export const dot3 = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
export const cross3 = (a, b) => [
  a[1] * b[2] - a[2] * b[1],
  a[2] * b[0] - a[0] * b[2],
  a[0] * b[1] - a[1] * b[0],
];
export const len3 = (a) => Math.hypot(a[0], a[1], a[2]);
export const dist3 = (a, b) => Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
export function normalize3(a) {
  const l = len3(a);
  return l > 0 ? [a[0] / l, a[1] / l, a[2] / l] : [0, 0, 0];
}

/** Viewport.tsx:193 — const dir = new THREE.Vector3(0.4, 0.4, 1).normalize(); */
export const FIT_DIR = normalize3(FIT_DIR_RAW);

/** local → world through the -PI/2-about-X scene group: (x,y,z) → (x, z, -y) */
export const localToWorld = (p) => [p[0], p[2], -p[1]];
/** world → local (Viewport.tsx:1457 worldToLocal on this group): (X,Y,Z) → (X, -Z, Y) */
export const worldToLocal = (p) => [p[0], -p[2], p[1]];
/** rotate a direction by the group rotation local → world: (x,y,z) → (x, z, -y) */
export const rotateGroup = (v) => [v[0], v[2], -v[1]];
/** rotate a direction by the inverse group rotation world → local: (X,Y,Z) → (X, -Z, Y) */
export const unrotateGroup = (v) => [v[0], -v[2], v[1]];

// ─── CameraFit (Viewport.tsx:165-217), transcribed ───────────────────────────

/**
 * Replicates CameraFit + R3F's default orthographic frustum.
 * R3F builds the ortho camera with left = -width/2, right = width/2,
 * top = height/2, bottom = -height/2 (CSS px), then CameraFit sets zoom,
 * position, near, far and calls updateProjectionMatrix().
 */
export function fitCamera(bbox, viewport) {
  const [min, max] = [bbox.min, bbox.max];
  // Viewport.tsx:177-182. STL bbox is Z-up. After the -PI/2 X rotation:
  // three.js Y = STL Z, three.js Z = -STL Y.
  const cx = (min[0] + max[0]) / 2;
  const cy = (min[2] + max[2]) / 2;
  const cz = -(min[1] + max[1]) / 2;
  const dimX = max[0] - min[0];
  const dimY = max[2] - min[2];
  const dimZ = max[1] - min[1];
  const maxDim = Math.max(dimX, dimY, dimZ);
  if (!(maxDim > 0)) throw new Error("fitCamera: degenerate bbox");

  const dir = FIT_DIR;
  const viewDist = maxDim * 10; // :194
  const position = [cx + dir[0] * viewDist, cy + dir[1] * viewDist, cz + dir[2] * viewDist]; // :195
  const target = [cx, cy, cz]; // :196 lookAt

  // :203-206
  const spanXZ = Math.max(dimX, dimZ);
  const effectiveDim = Math.max(maxDim, spanXZ * 1.3);
  const fit = Math.min(viewport.width, viewport.height) / effectiveDim;
  const zoom = Math.min(fit * 0.88, 500);
  // :215-216
  const near = Math.max(0.1, viewDist - maxDim * 6);
  const far = viewDist + maxDim * 6;

  // Camera basis exactly as THREE.Matrix4.lookAt(eye, target, up=(0,1,0)):
  //   zAxis = normalize(eye - target); xAxis = normalize(cross(up, zAxis));
  //   yAxis = cross(zAxis, xAxis)
  const zAxis = normalize3(sub3(position, target));
  const xAxis = normalize3(cross3([0, 1, 0], zAxis));
  const yAxis = cross3(zAxis, xAxis);

  return {
    width: viewport.width,
    height: viewport.height,
    position,
    target,
    dir,
    viewDist,
    near,
    far,
    zoom,
    // :1953 worldPerPx = (top-bottom)/zoom/height, with top-bottom === height
    worldPerPx: 1 / zoom,
    xAxis,
    yAxis,
    zAxis,
    // extra fit facts for the HUD
    dimX,
    dimY,
    dimZ,
    maxDim,
    spanXZ,
    effectiveDim,
    center: [cx, cy, cz],
    // R3F default symmetric ortho frustum
    frustum: {
      left: -viewport.width / 2,
      right: viewport.width / 2,
      top: viewport.height / 2,
      bottom: -viewport.height / 2,
    },
  };
}

/**
 * Build the camera model from an explicit pose (used by the page after an
 * orbit/zoom, and by the inset magnifier). Same basis convention as
 * THREE.Matrix4.lookAt / CameraFit.
 */
export function poseCamera({ position, target, zoom, width, height, near = 0.1, far = 1000 }) {
  const zAxis = normalize3(sub3(position, target));
  const xAxis = normalize3(cross3([0, 1, 0], zAxis));
  const yAxis = cross3(zAxis, xAxis);
  return {
    width,
    height,
    position,
    target,
    zoom,
    worldPerPx: 1 / zoom,
    near,
    far,
    viewDist: len3(sub3(position, target)),
    dir: zAxis,
    xAxis,
    yAxis,
    zAxis,
    frustum: { left: -width / 2, right: width / 2, top: height / 2, bottom: -height / 2 },
  };
}

/** Camera-space coordinates of a world point (view space, camera looks down -Z). */
export function worldToView(p, cam) {
  const v = sub3(p, cam.position);
  return [dot3(v, cam.xAxis), dot3(v, cam.yAxis), dot3(v, cam.zAxis)];
}

/**
 * World point → screen px in the canvas (y grows DOWN), matching
 * Vector3.project() under an ortho camera with the symmetric R3F frustum:
 *   px = zoom * viewX + width/2 ,  py = height/2 - zoom * viewY
 * (equivalently, worldPerPx = 1/zoom.)
 */
export function projectWorld(p, cam) {
  const [vx, vy] = worldToView(p, cam);
  return [cam.zoom * vx + cam.width / 2, cam.height / 2 - cam.zoom * vy];
}

/**
 * S1's projection: `seed.project(camera)` on the RAW local coordinates
 * (Viewport.tsx R6/R7 — Vector3.project() fed a model-local point).
 */
export function projectLocalAsWorld(localPoint, cam) {
  return projectWorld([localPoint[0], localPoint[1], localPoint[2]], cam);
}

/**
 * Raycaster.setFromCamera() for an OrthographicCamera (three r170):
 *   origin = (ndcX, ndcY, -(near+far)/(far-near)).unproject(camera)
 *          → on the camera plane: eye + xAxis*sx + yAxis*sy
 *   direction = (0,0,-1) transformed by camera.matrixWorld = -zAxis
 */
export function rayFromScreen(px, py, cam) {
  const ndcX = (px / cam.width) * 2 - 1;
  const ndcY = -((py / cam.height) * 2) + 1;
  const sx = (ndcX * (cam.width / 2)) / cam.zoom;
  const sy = (ndcY * (cam.height / 2)) / cam.zoom;
  const origin = add3(add3(cam.position, scale3(cam.xAxis, sx)), scale3(cam.yAxis, sy));
  const direction = scale3(cam.zAxis, -1);
  return { origin, direction };
}

/** Ray.distanceToPoint(): |cross(point - origin, direction)| for a unit direction. */
export function rayDistanceToPoint(ray, point) {
  return len3(cross3(sub3(point, ray.origin), ray.direction));
}

// ─── ray / triangle intersection (three.js Raycaster semantics) ──────────────

const TRI_EPS = 1e-12;

/**
 * Möller–Trumbore over the mesh's indexed triangle list, in WORLD space by
 * transforming the ray into model-local space (the scene group only rotates, so
 * lengths are preserved and `t` is the world-space distance).
 * Material side is DoubleSide in the app (Viewport.tsx:2589/2591), so no
 * back-face culling — matching three's raycaster.
 */
export function raycastMesh(ray, mesh) {
  const o = worldToLocal(ray.origin);
  const d = unrotateGroup(ray.direction); // linear map, stays unit-length
  const { vertices, faces } = mesh;
  let bestT = Infinity;
  let bestFace = -1;
  for (let f = 0, n = faces.length; f < n; f += 3) {
    const i0 = faces[f] * 3;
    const i1 = faces[f + 1] * 3;
    const i2 = faces[f + 2] * 3;
    const ax = vertices[i0], ay = vertices[i0 + 1], az = vertices[i0 + 2];
    const bx = vertices[i1], by = vertices[i1 + 1], bz = vertices[i1 + 2];
    const cx = vertices[i2], cy = vertices[i2 + 1], cz = vertices[i2 + 2];
    const e1x = bx - ax, e1y = by - ay, e1z = bz - az;
    const e2x = cx - ax, e2y = cy - ay, e2z = cz - az;
    const px = d[1] * e2z - d[2] * e2y;
    const py = d[2] * e2x - d[0] * e2z;
    const pz = d[0] * e2y - d[1] * e2x;
    const det = e1x * px + e1y * py + e1z * pz;
    if (Math.abs(det) < TRI_EPS) continue;
    const inv = 1 / det;
    const tx = o[0] - ax, ty = o[1] - ay, tz = o[2] - az;
    const u = (tx * px + ty * py + tz * pz) * inv;
    if (u < 0 || u > 1) continue;
    const qx = ty * e1z - tz * e1y;
    const qy = tz * e1x - tx * e1z;
    const qz = tx * e1y - ty * e1x;
    const v = (d[0] * qx + d[1] * qy + d[2] * qz) * inv;
    if (v < 0 || u + v > 1) continue;
    const t = (e2x * qx + e2y * qy + e2z * qz) * inv;
    if (t > TRI_EPS && t < bestT) {
      bestT = t;
      bestFace = f / 3;
    }
  }
  if (bestFace < 0) return null;
  const local = add3(o, scale3(d, bestT));
  return {
    faceIndex: bestFace,
    distance: bestT,
    local,
    world: localToWorld(local),
  };
}

/** Triangle centroid in model-local coordinates — Viewport.tsx:2065-2078. */
export function faceCentroid(mesh, faceIndex) {
  const fi = faceIndex * 3;
  const v0 = mesh.faces[fi], v1 = mesh.faces[fi + 1], v2 = mesh.faces[fi + 2];
  const j0 = v0 * 3, j1 = v1 * 3, j2 = v2 * 3;
  const vs = mesh.vertices;
  return [
    (vs[j0] + vs[j1] + vs[j2]) / 3,
    (vs[j0 + 1] + vs[j1 + 1] + vs[j2 + 1]) / 3,
    (vs[j0 + 2] + vs[j1 + 2] + vs[j2 + 2]) / 3,
  ];
}

// ─── derived sizes (Viewport.tsx:1059-1066) ─────────────────────────────────

export function seedMarkerSize(bbox) {
  const dx = bbox.max[0] - bbox.min[0];
  const dy = bbox.max[1] - bbox.min[1];
  const dz = bbox.max[2] - bbox.min[2];
  const diag = Math.hypot(dx, dy, dz);
  return {
    size: Math.max(SEED_MARKER_SIZE_MIN, diag * SEED_MARKER_SIZE_DIAG),
    diag,
  };
}

export function eraseThresholdWorld(size) {
  return size * ERASE_THRESHOLD_FACTOR;
}

// ─── glyph (Viewport.tsx:2704-2756) ─────────────────────────────────────────
//
// SEED_GEOMETRY = sphere(1,10,10), stem=cylinder(0.16,0.16,2,6), leaf=sphere(0.385,8,8)
// <group position={pos}>            ← OUTER group: seed stored model-local
//   <group ref={ref}>               ← INNER group: billboard quaternion copied here
//     sphere  scale=r               r = size
//     stem    scale=size position=[0, r + stemH/2, 0]   stemH = 2*size
//     leaf    scale=size position=[ leafR, r + stemH, 0] leafR = 0.55*size
//     leaf    scale=size position=[-leafR, r + stemH, 0]
// So in glyph-local units: sphere r=size, stem spans y∈[size,3size], leaves at
// y=3size, x=±0.55size, leaf r=0.385size.

export function glyphPoints(size) {
  return {
    center: [0, 0, 0],
    stemTop: [0, 3 * size, 0],
    leafL: [-0.55 * size, 3 * size, 0],
    leafR: [0.55 * size, 3 * size, 0],
    // extent of the sapling in glyph-local units
    maxRadius: Math.hypot(0.55 * size + 0.385 * size, 3 * size),
  };
}

/** Apply the camera's world orientation to a glyph-local vector. */
export function cameraFrameApply(cam, v) {
  return add3(
    add3(scale3(cam.xAxis, v[0]), scale3(cam.yAxis, v[1])),
    scale3(cam.zAxis, v[2])
  );
}

/**
 * The billboard bug (Viewport.tsx:2786-2792) measured:
 *   g.quaternion.copy(cam.quaternion)   ← sets the CHILD's LOCAL quaternion to
 *                                         the camera's WORLD quaternion.
 * The child sits under a group rotated -PI/2 about X, so the glyph's world
 * orientation is R_x(-90°) · q_camera, i.e. exactly 90° away from the camera.
 * A correct billboard needs the CHILD's local q to be parentWorld⁻¹ · camWorld.
 *
 * Returns the glyph's world frame in both modes plus the measured tilt angle.
 */
export function glyphWorldFrame(cam, mode = "bug") {
  const apply = (v) => (mode === "bug" ? rotateGroup(cameraFrameApply(cam, v)) : cameraFrameApply(cam, v));
  const bx = apply([1, 0, 0]);
  const by = apply([0, 1, 0]);
  const bz = apply([0, 0, 1]);
  // angle between this frame and the camera frame: trace(R) = Σ e'_i · e_i
  const trace = dot3(bx, cam.xAxis) + dot3(by, cam.yAxis) + dot3(bz, cam.zAxis);
  const tiltDeg = (Math.acos(Math.max(-1, Math.min(1, (trace - 1) / 2))) * 180) / Math.PI;
  return { x: bx, y: by, z: bz, tiltDeg };
}

/** World-space offset of a glyph point from the seed position. */
export function glyphPointOffset(cam, glyphLocal, mode = "bug") {
  return mode === "bug" ? rotateGroup(cameraFrameApply(cam, glyphLocal)) : cameraFrameApply(cam, glyphLocal);
}

// ─── strategy table ─────────────────────────────────────────────────────────

export const STRATEGIES = [
  {
    id: "S1",
    label: "project(local seed) → screen",
    source: "R6/R7 click test (Viewport.tsx ~1939-1968)",
    detail:
      "seedVec.set(s.x, s.y, s.z) then Vector3.project(camera) — the MODEL-LOCAL seed is fed to a world-space projection.",
    metric: "px",
    unit: "px",
    expected: "FAIL",
  },
  {
    id: "S2",
    label: "project(localToWorld(seed)) → screen",
    source: "corrected R6/R7 (group rotation applied first)",
    detail:
      "p_world = (x, z, -y); Vector3.project(camera) on the WORLD position — agrees with the rendered marker.",
    metric: "px",
    unit: "px",
    expected: "PASS",
  },
  {
    id: "S3",
    label: "ray.distanceToPoint(local seed)",
    source: "R8-first (Viewport.tsx ~1950-1963)",
    detail:
      "ray is in WORLD space; the seed is MODEL-LOCAL. dPx = d / worldPerPx, threshold 32 px.",
    metric: "px",
    unit: "px",
    expected: "FAIL",
  },
  {
    id: "S4",
    label: "ray.distanceToPoint(world seed)",
    source: "corrected R8-first",
    detail: "same 32 px test, but the ray and the seed are in the same (world) space.",
    metric: "px",
    unit: "px",
    expected: "PASS",
  },
  {
    id: "S5",
    label: "worldToLocal(hit).distanceTo(local seed)",
    source: "R8-FINAL click path (Viewport.tsx :1991-1992 → eraseNearWorldPoint :1076)",
    detail:
      "getLocalHit() converts the world hit to model-local, and seeds are local → consistent. Threshold = seedMarkerSize*2.5 world units.",
    metric: "wu",
    unit: "wu",
    expected: "PASS",
  },
  {
    id: "S6",
    label: "hit(world).distanceTo(local seed)",
    source: "R8-FINAL smear path (Viewport.tsx :2125-2126)",
    detail:
      "raycastFace() returns a WORLD-space point, eraseNearWorldPoint() compares it with MODEL-LOCAL seeds → mixed spaces, distance is never below the threshold.",
    metric: "wu",
    unit: "wu",
    expected: "FAIL",
  },
];

export function strategyThreshold(strategy, markerSize, cam) {
  return strategy.id === "S5" || strategy.id === "S6"
    ? eraseThresholdWorld(markerSize)
    : SCREEN_THRESHOLD_PX;
}

// ─── one click, all six strategies ──────────────────────────────────────────

/**
 * Evaluate every strategy for one click.
 *
 * @param {object} args
 * @param {[number,number]} args.clickPx  canvas-local CSS px (mouse position)
 * @param {object} args.cam               from fitCamera (screen px mapping)
 * @param {object} args.mesh              {vertices, faces}
 * @param {number[]} args.seedsLocal      seed positions, MODEL-LOCAL triples
 * @param {number} args.markerSize
 * @param {number|null} args.intended     index of the seed the click was aimed at
 */
export function evaluateClick({ clickPx, cam, mesh, seedsLocal, markerSize, intended = null }) {
  const ray = rayFromScreen(clickPx[0], clickPx[1], cam);
  const hit = raycastMesh(ray, mesh);
  return evaluateFromHit({ clickPx, cam, seedsLocal, markerSize, intended, hit });
}

/**
 * The six strategies, given an already-computed surface hit. The browser hands
 * in the hit produced by the REAL three.js Raycaster (Viewport.tsx:1791-1802 /
 * getLocalHit :1425-1462), so the table reports what the app's own engine does.
 *
 * @param {object} args
 * @param {object|null} args.hit  {faceIndex, world:[x,y,z], local:[x,y,z]} or null
 */
export function evaluateFromHit({ clickPx, cam, seedsLocal, markerSize, intended = null, hit = null }) {
  const ray = rayFromScreen(clickPx[0], clickPx[1], cam);
  const seedsWorld = seedsLocal.map((s) => localToWorld(s));
  const threshold = eraseThresholdWorld(markerSize);

  // Per-strategy raw values, evaluated like the app does: nearest seed wins,
  // then the threshold decides.
  const values = {
    S1: seedsLocal.map((s) => {
      const p = projectLocalAsWorld(s, cam);
      return Math.hypot(p[0] - clickPx[0], p[1] - clickPx[1]);
    }),
    S2: seedsWorld.map((w) => {
      const p = projectWorld(w, cam);
      return Math.hypot(p[0] - clickPx[0], p[1] - clickPx[1]);
    }),
    S3: seedsLocal.map((s) => rayDistanceToPoint(ray, [s[0], s[1], s[2]]) / cam.worldPerPx),
    S4: seedsWorld.map((w) => rayDistanceToPoint(ray, w) / cam.worldPerPx),
    S5: hit ? seedsLocal.map((s) => dist3(hit.local, s)) : null,
    S6: hit ? seedsLocal.map((s) => dist3(hit.world, s)) : null,
  };

  const rows = STRATEGIES.map((strategy) => {
    const vals = values[strategy.id];
    const thr = strategyThreshold(strategy, markerSize, cam);
    if (!vals) {
      return {
        ...strategy,
        value: null,
        threshold: thr,
        nearestSeed: null,
        pass: false,
        strict: false,
        unavailable: "no mesh hit at this pixel — raycastFace/getLocalHit return null, the app returns early",
      };
    }
    let nearestSeed = 0;
    let value = Infinity;
    for (let i = 0; i < vals.length; i++) {
      if (vals[i] < value) {
        value = vals[i];
        nearestSeed = i;
      }
    }
    if (vals.length === 0) {
      nearestSeed = null;
      value = Infinity;
    }
    const pass = vals.length > 0 && value <= thr;
    return {
      ...strategy,
      value,
      threshold: thr,
      nearestSeed,
      pass,
      strict: pass && (intended == null || nearestSeed === intended),
    };
  });

  // ── overlay geometry (snapshot; stays valid even if the seed is erased) ──
  const seedIndex = intended != null ? intended : rows.find((r) => r.id === "S1")?.nearestSeed ?? 0;
  const seed = seedsLocal[seedIndex] ?? seedsLocal[0] ?? null;
  const g = glyphPoints(markerSize);
  const overlay = { seedIndex, seedLocal: seed, seedWorld: seed ? localToWorld(seed) : null };
  if (seed) {
    overlay.renderedPx = projectWorld(localToWorld(seed), cam); // where the marker IS drawn
    overlay.phantomPx = projectLocalAsWorld(seed, cam); // where S1 thinks it is
    overlay.eraseRadiusPx = threshold * cam.zoom;
    overlay.glyph = {};
    for (const mode of ["bug", "correct"]) {
      const pts = {
        center: glyphPointOffset(cam, g.center, mode),
        stemTop: glyphPointOffset(cam, g.stemTop, mode),
        leafL: glyphPointOffset(cam, g.leafL, mode),
        leafR: glyphPointOffset(cam, g.leafR, mode),
      };
      const px = {};
      let maxR = 0;
      for (const [k, off] of Object.entries(pts)) {
        px[k] = projectWorld(add3(overlay.seedWorld, off), cam);
        maxR = Math.max(maxR, Math.hypot(px[k][0] - overlay.renderedPx[0], px[k][1] - overlay.renderedPx[1]));
      }
      overlay.glyph[mode] = { px, radiusPx: maxR, tiltDeg: glyphWorldFrame(cam, mode).tiltDeg };
    }
  }

  return {
    clickPx: [clickPx[0], clickPx[1]],
    intended,
    ray,
    hit,
    worldPerPx: cam.worldPerPx,
    seedsLocal,
    seedsWorld,
    markerSize,
    eraseThresholdWorld: threshold,
    rows,
    overlay,
  };
}

// ─── models (synthetic, Z-up STL layout, flat arrays) ───────────────────────

function bboxOf(vertices) {
  const min = [Infinity, Infinity, Infinity];
  const max = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < vertices.length; i += 3) {
    for (let k = 0; k < 3; k++) {
      if (vertices[i + k] < min[k]) min[k] = vertices[i + k];
      if (vertices[i + k] > max[k]) max[k] = vertices[i + k];
    }
  }
  return { min, max };
}

/** Build a mesh object from flat arrays (also used by the page's STL loader). */
export function meshFromArrays(name, label, vertices, faces, note = "") {
  return finishModel(name, label, vertices, faces, note);
}

function finishModel(name, label, vertices, faces, note) {
  return {
    name,
    label,
    vertices,
    faces,
    faceCount: faces.length / 3,
    bbox: bboxOf(vertices),
    note,
  };
}

/** Box: 40 × 40 × 40, sitting on the STL Z=0 plane. 12 triangles. */
export function buildBox() {
  const v = [
    [-20, -20, 0], [20, -20, 0], [20, 20, 0], [-20, 20, 0],
    [-20, -20, 40], [20, -20, 40], [20, 20, 40], [-20, 20, 40],
  ];
  const vertices = v.flat();
  const faces = [
    0, 2, 1, 0, 3, 2, // bottom -Z
    4, 5, 6, 4, 6, 7, // top +Z
    0, 1, 5, 0, 5, 4, // -Y
    1, 2, 6, 1, 6, 5, // +X
    2, 3, 7, 2, 7, 6, // +Y
    3, 0, 4, 3, 4, 7, // -X
  ];
  return finishModel("box", "Box · 12 tris", vertices, faces,
    "flat box on the STL Z=0 plane; every face is huge, so the face centroid is far from the model centre — the clearest possible early failure for S1/S6.");
}

/**
 * Dome: hemisphere R=20 on the STL Z=0 plane + base disc. 24 rings × 40
 * segments → 1 920 triangles (≈2k, per the spec).
 */
export function buildDome() {
  const R = 20;
  const SEG = 40;
  const RINGS = 24;
  const vertices = [];
  for (let i = 0; i < RINGS; i++) {
    const phi = (i / RINGS) * (Math.PI / 2);
    const r = R * Math.cos(phi);
    const z = R * Math.sin(phi);
    for (let j = 0; j < SEG; j++) {
      const th = (j / SEG) * Math.PI * 2;
      vertices.push(r * Math.cos(th), r * Math.sin(th), z);
    }
  }
  const pole = vertices.length / 3; // (0,0,R)
  vertices.push(0, 0, R);
  const baseCenter = vertices.length / 3; // (0,0,0)
  vertices.push(0, 0, 0);

  const faces = [];
  for (let i = 0; i < RINGS - 1; i++) {
    for (let j = 0; j < SEG; j++) {
      const j2 = (j + 1) % SEG;
      const a = i * SEG + j;
      const b = i * SEG + j2;
      const c = (i + 1) * SEG + j2;
      const d = (i + 1) * SEG + j;
      faces.push(a, b, c, a, c, d);
    }
  }
  const last = (RINGS - 1) * SEG;
  for (let j = 0; j < SEG; j++) {
    const j2 = (j + 1) % SEG;
    faces.push(last + j, last + j2, pole); // cap fan, outward
    faces.push(baseCenter, j2, j); // base disc, -Z
  }
  return finishModel("dome", "Dome · 1 920 tris", vertices, faces,
    "hemisphere with a flat base: convex, so the frontmost hit at a marker's pixel is the marker's own face — a clean S5 baseline.");
}

/**
 * Torus knot: three.js TorusKnotGeometry(r=20, tube=3.4, 400, 60, p=2, q=3),
 * laid out around the STL Z axis (the app's own convention). 48 000 triangles.
 */
export function buildTorusKnot() {
  const radius = 20;
  const tube = 3.4;
  const tubularSegments = 400;
  const radialSegments = 60;
  const p = 2;
  const q = 3;

  const posOnCurve = (u, out) => {
    const cu = Math.cos(u);
    const su = Math.sin(u);
    const quOverP = (q / p) * u;
    const cs = Math.cos(quOverP);
    out[0] = radius * (2 + cs) * 0.5 * cu;
    out[1] = radius * (2 + cs) * su * 0.5;
    out[2] = radius * Math.sin(quOverP) * 0.5;
  };

  const vertices = [];
  const faces = [];
  const P1 = [0, 0, 0];
  const P2 = [0, 0, 0];
  const T = [0, 0, 0];
  const N = [0, 0, 0];
  const B = [0, 0, 0];

  const stride = radialSegments + 1;
  for (let i = 0; i <= tubularSegments; i++) {
    const u = (i / tubularSegments) * p * Math.PI * 2;
    posOnCurve(u, P1);
    posOnCurve(u + 0.01, P2);
    T[0] = P2[0] - P1[0]; T[1] = P2[1] - P1[1]; T[2] = P2[2] - P1[2];
    N[0] = P2[0] + P1[0]; N[1] = P2[1] + P1[1]; N[2] = P2[2] + P1[2];
    B[0] = T[1] * N[2] - T[2] * N[1];
    B[1] = T[2] * N[0] - T[0] * N[2];
    B[2] = T[0] * N[1] - T[1] * N[0];
    N[0] = B[1] * T[2] - B[2] * T[1];
    N[1] = B[2] * T[0] - B[0] * T[2];
    N[2] = B[0] * T[1] - B[1] * T[0];
    normalize3InPlace(B);
    normalize3InPlace(N);
    for (let j = 0; j <= radialSegments; j++) {
      const vv = (j / radialSegments) * Math.PI * 2;
      const ccx = -tube * Math.cos(vv);
      const ccy = Math.sin(vv);
      vertices.push(
        P1[0] + ccx * N[0] + ccy * B[0],
        P1[1] + ccx * N[1] + ccy * B[1],
        P1[2] + ccx * N[2] + ccy * B[2]
      );
    }
  }
  for (let j = 1; j <= tubularSegments; j++) {
    for (let i = 1; i <= radialSegments; i++) {
      const a = stride * (j - 1) + (i - 1);
      const b = stride * j + (i - 1);
      const c = stride * j + i;
      const d = stride * (j - 1) + i;
      faces.push(a, b, d, b, c, d);
    }
  }
  return finishModel("knot", "Torus knot · 48 000 tris", vertices, faces,
    "three.js TorusKnotGeometry point-for-point, around the STL Z axis — the high-face-count case where the marker glyph is a handful of pixels.");
}

function normalize3InPlace(a) {
  const l = Math.hypot(a[0], a[1], a[2]) || 1;
  a[0] /= l; a[1] /= l; a[2] /= l;
}

export const MODELS = [
  { id: "dome", label: "Dome · 1 920 tris", build: buildDome },
  { id: "box", label: "Box · 12 tris", build: buildBox },
  { id: "knot", label: "Torus knot · 48 000 tris", build: buildTorusKnot },
];

export function buildModel(id) {
  const entry = MODELS.find((m) => m.id === id);
  if (!entry) throw new Error(`unknown model: ${id}`);
  return entry.build();
}

// ─── deterministic seed placement ───────────────────────────────────────────

/**
 * Place seeds the way a user does: click somewhere on the visible model, take
 * the face centroid (Viewport.tsx:2065-2078). Candidates come from a fixed
 * screen grid, so placement is fully deterministic.
 *
 * `raycastAt(px, py)` may be injected: the browser passes the REAL three.js
 * Raycaster (the app's own path) while probe.mjs uses core's Möller–Trumbore.
 *
 * A candidate is only kept if a click at ITS OWN rounded projected pixel comes
 * back within the S5 threshold of the centroid — i.e. only seeds that are
 * actually clickable in the current view are placed. (A seed the user can see
 * but cannot hit would be a separate, view-dependent artefact; the self-test
 * must isolate the coordinate-space bug.)
 */
export function autoPlaceSeeds({ mesh, cam, count = 5, markerSize, raycastAt = null }) {
  const threshold = eraseThresholdWorld(markerSize);
  const raycast = raycastAt || ((px, py) => raycastMesh(rayFromScreen(px, py, cam), mesh));
  const COLS = 13;
  const ROWS = 9;
  const candidates = [];
  const seenFaces = new Set();

  for (let r = 1; r <= ROWS; r++) {
    for (let c = 1; c <= COLS; c++) {
      const px = (c / (COLS + 1)) * cam.width;
      const py = (r / (ROWS + 1)) * cam.height;
      const hit = raycast(px, py);
      if (!hit) continue;
      if (seenFaces.has(hit.faceIndex)) continue;
      seenFaces.add(hit.faceIndex);
      const centroid = faceCentroid(mesh, hit.faceIndex);
      const proj = projectWorld(localToWorld(centroid), cam);
      const testPx = [Math.round(proj[0]), Math.round(proj[1])];
      if (testPx[0] < 1 || testPx[1] < 1 || testPx[0] > cam.width - 1 || testPx[1] > cam.height - 1) continue;
      const verify = raycast(testPx[0], testPx[1]);
      if (!verify) continue;
      const d = dist3(verify.local, centroid);
      if (d > threshold * 0.8) continue; // not cleanly clickable from this view
      candidates.push({ centroid, clickPx: testPx, checkD: d });
    }
  }

  // Greedy max-min spread, deterministic (ties → lowest grid order).
  const picked = [];
  if (candidates.length > 0) picked.push(0);
  while (picked.length < count && picked.length < candidates.length) {
    let best = -1;
    let bestScore = -1;
    for (let i = 0; i < candidates.length; i++) {
      if (picked.includes(i)) continue;
      let minD = Infinity;
      for (const pi of picked) {
        minD = Math.min(minD, dist3(candidates[i].centroid, candidates[pi].centroid));
      }
      if (minD > bestScore) {
        bestScore = minD;
        best = i;
      }
    }
    if (best < 0) break;
    picked.push(best);
  }
  return picked.map((i, index) => ({
    index,
    local: candidates[i].centroid,
    world: localToWorld(candidates[i].centroid),
    placedAtPx: candidates[i].clickPx,
    checkDistance: candidates[i].checkD,
  }));
}

// ─── aggregation + report ───────────────────────────────────────────────────
//
// Verdict rules (also in README):
//   PASS = for EVERY click the strategy's nearest seed is the intended one AND
//          the value is within that strategy's own threshold.
//   FAIL = otherwise (a wrong seed wins, or the distance exceeds the threshold).
// The expected column is the strategy's own space-correctness, so the table
// proves which code paths can ever erase.

/**
 * The two aim scenarios. A human aiming at a sapling glyph is never exact, and
 * the S5 tolerance is only a couple of pixels at the fitted zoom, so the
 * "4 px off" scenario is what the bug report actually feels like.
 */
const CENTRED_EXPECT = { S1: "FAIL", S2: "PASS", S3: "FAIL", S4: "PASS", S5: "PASS", S6: "FAIL" };
const OFFSET4_EXPECT = { ...CENTRED_EXPECT, S5: "FAIL" };

export const SCENARIOS = [
  {
    id: "centred",
    label: "centred — click on the rendered marker (integer pixel of S2)",
    offsetPx: [0, 0],
    expected: CENTRED_EXPECT,
    // why each verdict is expected in THIS scenario
    expectedNote: {
      S5: "exact aim: the surface hit is inside the 2.5x size tolerance",
    },
  },
  {
    id: "offset4",
    label: "4 px off — click 4 px right of the rendered marker (human aim)",
    offsetPx: [4, 0],
    expected: OFFSET4_EXPECT,
    expectedNote: {
      S5: "4 px aim error exceeds the S5 tolerance, so HEAD's click path cannot erase",
    },
  },
];

function seedLocalOf(seed, index, seedsLocal) {
  return Array.isArray(seed) ? seed : seed.local ?? seedsLocal[index];
}
function seedWorldOf(seed, index, seedsLocal) {
  if (Array.isArray(seed)) return localToWorld(seed);
  return seed.world ?? localToWorld(seedLocalOf(seed, index, seedsLocal));
}

/**
 * Evaluate every strategy for every scenario. `raycastAt(px,py)` may be injected
 * (the page passes the real three.js Raycaster); it must return
 * {faceIndex, world:[x,y,z], local:[x,y,z]} or null.
 */
export function evaluateScenarios({ mesh, cam, seeds, markerSize, raycastAt = null, scenarios = SCENARIOS }) {
  const seedsLocal = seeds.map((s, i) => seedLocalOf(s, i, []).slice());
  const raycast = raycastAt || ((px, py) => raycastMesh(rayFromScreen(px, py, cam), mesh));
  return scenarios.map((sc) => ({
    id: sc.id,
    label: sc.label,
    offsetPx: sc.offsetPx.slice(),
    expected: sc.expected,
    expectedNote: sc.expectedNote ?? {},
    clicks: seeds.map((seed, i) => {
      const projected = projectWorld(seedWorldOf(seed, i, seedsLocal), cam);
      const clickPx = [
        Math.round(projected[0]) + sc.offsetPx[0],
        Math.round(projected[1]) + sc.offsetPx[1],
      ];
      const hit = raycast(clickPx[0], clickPx[1]);
      const evaluation = evaluateFromHit({
        clickPx,
        cam,
        seedsLocal,
        markerSize,
        intended: i,
        hit,
      });
      return { seedIndex: i, clickPx, projected, offsetPx: sc.offsetPx.slice(), evaluation };
    }),
  }));
}

export function aggregateClicks(clicks, expectedMap = CENTRED_EXPECT, expectedNote = {}) {
  return STRATEGIES.map((strategy) => {
    const expected = expectedMap[strategy.id] ?? strategy.expected;
    const rows = clicks.map((c, clickIndex) => ({
      clickIndex,
      intended: c.seedIndex,
      ...c.evaluation.rows.find((r) => r.id === strategy.id),
    }));
    const withValue = rows.filter((r) => r.value != null && Number.isFinite(r.value));
    const strictOk = rows.every((r) => r.strict);
    const passOk = rows.every((r) => r.pass);
    const wrongSeed = rows.filter((r) => r.value != null && r.nearestSeed !== r.intended).length;
    const minValue = withValue.length ? Math.min(...withValue.map((r) => r.value)) : null;
    const maxValue = withValue.length ? Math.max(...withValue.map((r) => r.value)) : null;
    const actual = strictOk ? "PASS" : passOk ? "PASS*" : "FAIL";
    return {
      id: strategy.id,
      label: strategy.label,
      source: strategy.source,
      detail: strategy.detail,
      metric: strategy.metric,
      unit: strategy.unit,
      expected,
      expectedNote: expectedNote[strategy.id] ?? "",
      baselineExpected: strategy.expected,
      actual,
      ok: actual === expected,
      passedClicks: rows.filter((r) => r.strict).length,
      totalClicks: rows.length,
      wrongSeed,
      minValue,
      maxValue,
      threshold: rows[0]?.threshold ?? null,
      toleranceText: rows[0]?.toleranceText ?? null,
      perClick: rows.map((r) => ({
        clickIndex: r.clickIndex,
        intended: r.intended,
        value: r.value,
        threshold: r.threshold,
        toleranceText: r.toleranceText ?? null,
        nearestSeed: r.nearestSeed,
        pass: r.pass,
        strict: r.strict,
        unavailable: r.unavailable ?? null,
      })),
    };
  });
}

/**
 * Assemble the forensic report. `seeds` may be autoPlaceSeeds() objects or plain
 * local triples; `scenarios` comes from evaluateScenarios().
 */
export function buildReport({ mesh, cam, viewport, seeds, scenarios = null, clicks = null }) {
  const { size, diag } = seedMarkerSize(mesh.bbox);
  const threshold = eraseThresholdWorld(size);
  const s5TolerancePx = threshold * cam.zoom;

  // Backwards-compatible single-scenario input.
  const scens =
    scenarios ||
    [
      {
        id: "centred",
        label: SCENARIOS[0].label,
        offsetPx: [0, 0],
        clicks: clicks || [],
      },
    ];
  for (const sc of scens) {
    sc.aggregate = aggregateClicks(sc.clicks, sc.expected, sc.expectedNote);
    sc.mismatches = sc.aggregate.filter((a) => !a.ok).map((a) => `${a.id}: expected ${a.expected}, actual ${a.actual}`);
    sc.allOk = sc.mismatches.length === 0;
  }
  const mismatches = scens.flatMap((sc) => sc.mismatches.map((m) => `[${sc.id}] ${m}`));

  const report = {
    schema: "coloryourmodel/seed-erase-forensics/2",
    generatedAt: new Date().toISOString(),
    source: {
      repo: "ColorYourModel",
      branch: "feature_012_governance",
      head: "64d544e",
      file: "src/components/Viewport/Viewport.tsx",
      groupRotation: "-PI/2 about X (Viewport.tsx:2580)",
      seedStorage: "model-local face centroid (Viewport.tsx:2065-2078, src/types/mesh.ts:42-49)",
    },
    model: {
      id: mesh.name,
      label: mesh.label,
      faceCount: mesh.faceCount,
      bbox: mesh.bbox,
      note: mesh.note ?? "",
    },
    viewport: { ...viewport },
    camera: {
      position: cam.position,
      target: cam.target,
      dir: cam.dir,
      zoom: cam.zoom,
      worldPerPx: cam.worldPerPx,
      near: cam.near,
      far: cam.far,
      maxDim: cam.maxDim ?? null,
      spanXZ: cam.spanXZ ?? null,
      effectiveDim: cam.effectiveDim ?? null,
    },
    sizes: {
      diag,
      seedMarkerSize: size,
      eraseThresholdWorld: threshold,
      eraseThresholdPx: s5TolerancePx,
      s5TolerancePx,
      screenThresholdPx: SCREEN_THRESHOLD_PX,
      glyphTiltBugDeg: glyphWorldFrame(cam, "bug").tiltDeg,
      glyphTiltCorrectDeg: glyphWorldFrame(cam, "correct").tiltDeg,
    },
    seeds: seeds.map((s, index) => {
      const local = seedLocalOf(s, index, []);
      const world = seedWorldOf(s, index, []);
      return {
        index: s.index ?? index,
        local,
        world,
        projectedPx: projectWorld(world, cam),
        placedAtPx: s.placedAtPx ?? null,
        checkDistance: s.checkDistance ?? null,
      };
    }),
    scenarios: scens.map((sc) => ({
      id: sc.id,
      label: sc.label,
      offsetPx: sc.offsetPx,
      expected: sc.expected,
      expectedNote: sc.expectedNote ?? {},
      clicks: sc.clicks.map((c) => ({
        seedIndex: c.seedIndex,
        clickPx: c.clickPx,
        projectedPx: c.projected ?? null,
        hit: c.evaluation.hit
          ? {
              faceIndex: c.evaluation.hit.faceIndex,
              world: c.evaluation.hit.world,
              local: c.evaluation.hit.local,
            }
          : null,
        rows: c.evaluation.rows.map((r) => ({
          id: r.id,
          label: r.label,
          metric: r.metric,
          value: r.value,
          threshold: r.threshold,
          toleranceText: r.toleranceText ?? null,
          nearestSeed: r.nearestSeed,
          intended: c.seedIndex,
          pass: r.pass,
          strict: r.strict,
          unit: r.unit,
          unavailable: r.unavailable ?? null,
        })),
        overlay: {
          renderedPx: c.evaluation.overlay.renderedPx ?? null,
          phantomPx: c.evaluation.overlay.phantomPx ?? null,
          eraseRadiusPx: c.evaluation.overlay.eraseRadiusPx ?? null,
          glyphRadiusPx: c.evaluation.overlay.glyph?.bug.radiusPx ?? null,
          glyphTiltBugDeg: c.evaluation.overlay.glyph?.bug.tiltDeg ?? null,
          glyphTiltCorrectDeg: c.evaluation.overlay.glyph?.correct.tiltDeg ?? null,
        },
      })),
      aggregate: sc.aggregate,
      allOk: sc.allOk,
      mismatches: sc.mismatches,
    })),
    aggregate: scens[0].aggregate, // headline (centred) scenario
    clicks: scens[0].clicks,
    expectedVsActual: {
      allOk: mismatches.length === 0,
      mismatches,
      headline: scens[0].allOk,
    },
  };
  report.text = formatReport(report);
  return report;
}

/** Headless convenience: place + click with core's raycast (no three.js). */
export function runSelfTest({ modelId = "dome", viewport = CANONICAL_VIEWPORT, seedCount = 5, mesh: providedMesh } = {}) {
  const mesh = providedMesh ?? buildModel(modelId);
  return selfTestWith({ mesh, viewport, seedCount });
}

/**
 * Shared self-test driver. `raycastAt` lets the page inject three.js's
 * Raycaster so the numbers come from the app's real engine.
 */
export function selfTestWith({ mesh, viewport = CANONICAL_VIEWPORT, seedCount = 5, raycastAt = null }) {
  const cam = fitCamera(mesh.bbox, viewport);
  const { size } = seedMarkerSize(mesh.bbox);
  const seeds = autoPlaceSeeds({ mesh, cam, count: seedCount, markerSize: size, raycastAt });
  const scenarios = evaluateScenarios({ mesh, cam, seeds, markerSize: size, raycastAt });
  return buildReport({ mesh, cam, viewport, seeds, scenarios });
}

// ─── text formatting (probe.mjs stdout + browser console) ───────────────────

const padR = (s, n) => String(s).padStart(n);
const padL = (s, n) => String(s).padEnd(n);

function fmt(n, digits = 2) {
  if (n === null || n === undefined || !Number.isFinite(n)) return "—";
  const s = n.toFixed(digits);
  return s === "-0.00" ? "0.00" : s;
}

function triple(t, digits = 2) {
  if (!t) return "—";
  return `(${t.map((v) => padR(fmt(v, digits), 8)).join(", ")})`;
}

function toleranceText(row, sizes) {
  if (!row) return "—";
  if (row.unit === "px") return `${fmt(row.threshold, 1)} px`;
  return `${fmt(row.threshold, 4)} wu ≈ ${fmt(sizes.s5TolerancePx, 2)} px`;
}

function clickLine(click, sizes) {
  const parts = click.rows.map((r) => {
    const tol = r.unit === "px" ? `${fmt(r.threshold, 1)}px` : `${fmt(r.threshold, 4)}wu≈${fmt(sizes.s5TolerancePx, 2)}px`;
    const val = r.value == null ? "—" : fmt(r.value, r.unit === "px" ? 1 : 4);
    const verdict = r.unavailable ? "N/A" : r.strict ? "PASS" : r.pass ? "PASS?" : "FAIL";
    return `${r.id} ${verdict} ${val}${r.unit} (thr ${tol})`;
  });
  return parts.join("  ");
}

export function formatReport(report) {
  const L = [];
  const S = report.sizes;
  const bar = "═".repeat(118);
  L.push(bar);
  L.push(" ColorYourModel — seed-erase forensic probe  (tools/seed-erase-forensics)");
  L.push(` model: ${report.model.label}   faces=${report.model.faceCount}`);
  L.push(` bbox (STL/local, Z-up): min=${triple(report.model.bbox.min)} max=${triple(report.model.bbox.max)}`);
  L.push(` viewport: ${report.viewport.width}x${report.viewport.height} px   zoom=${fmt(report.camera.zoom, 3)}   worldPerPx=${fmt(report.camera.worldPerPx, 5)}`);
  L.push(` camera eye=${triple(report.camera.position)} target=${triple(report.camera.target)}   dir=${triple(report.camera.dir, 3)}`);
  L.push(` fit: maxDim=${fmt(report.camera.maxDim, 2)} spanXZ=${fmt(report.camera.spanXZ, 2)} effectiveDim=${fmt(report.camera.effectiveDim, 2)} near=${fmt(report.camera.near, 1)} far=${fmt(report.camera.far, 1)}`);
  L.push(` seedMarkerSize=${fmt(S.seedMarkerSize, 5)}  S5 tolerance = size*2.5 = ${fmt(S.eraseThresholdWorld, 5)} wu = ${fmt(S.s5TolerancePx, 2)} px   S1..S4 threshold = 32 px`);
  L.push(` glyph billboard tilt: bug=${fmt(S.glyphTiltBugDeg, 3)}°  corrected=${fmt(S.glyphTiltCorrectDeg, 3)}°`);
  L.push(" transform: group rotation -PI/2 about X  =>  world = (x, z, -y)   local = (X, -Z, Y)");
  L.push(bar);
  L.push("");
  L.push("EXPECTED vs ACTUAL — both aim scenarios (S5 tolerance shown in px)");
  for (const sc of report.scenarios) {
    L.push("");
    L.push(` ── ${sc.id}: ${sc.label} ──`);
    L.push(
      ` ${padL("strategy", 42)} ${padL("code path", 34)} ${padR("tolerance", 24)} ${padR("expected", 9)} ${padR("actual", 7)} ${padR("hits", 6)}  evidence`
    );
    for (const a of sc.aggregate) {
      const hits = `${a.passedClicks}/${a.totalClicks}`;
      const tol = a.unit === "px" ? `${fmt(a.threshold, 1)} px` : `${fmt(a.threshold, 4)} wu ≈ ${fmt(S.s5TolerancePx, 2)} px`;
      const evidence =
        (a.expected === "FAIL"
          ? `${a.wrongSeed}/${a.totalClicks} clicks chose a wrong seed; best d=${fmt(a.minValue, a.unit === "px" ? 1 : 4)} ${a.unit}`
          : `worst d=${fmt(a.maxValue, a.unit === "px" ? 1 : 4)} ${a.unit}`) +
        (a.expectedNote ? `  [${a.expectedNote}]` : "");
      L.push(
        ` ${padL(`${a.id} ${a.label}`, 42)} ${padL(a.source.length > 33 ? a.source.slice(0, 33) : a.source, 34)} ${padR(tol, 24)} ${padR(a.expected, 9)} ${padR(a.actual, 7)} ${padR(hits, 6)}  ${evidence}`
      );
    }
    if (!sc.allOk) L.push(`   !! ${sc.mismatches.join(" | ")}`);
  }
  L.push("");
  L.push("PER-CLICK detail (every strategy, every click, both scenarios)");
  for (const sc of report.scenarios) {
    L.push("");
    L.push(` ── ${sc.id} (offset ${sc.offsetPx[0]} px, ${sc.offsetPx[1]} px) ──`);
    for (const click of sc.clicks) {
      const hitTxt = click.hit
        ? `face ${click.hit.faceIndex} world=${triple(click.hit.world)} local=${triple(click.hit.local)}`
        : "MISS (getLocalHit → null)";
      L.push(` click ${click.seedIndex + 1}/${sc.clicks.length}  seed#${click.seedIndex}  clickPx=(${fmt(click.clickPx[0], 0)}, ${fmt(click.clickPx[1], 0)})  ${hitTxt}`);
      L.push(`   seed local=${triple(report.seeds[click.seedIndex].local)}`);
      L.push(`   ${clickLine(click, S)}`);
    }
  }
  L.push("");
  L.push(
    report.expectedVsActual.allOk
      ? ` SELF-TEST: PASS — every strategy behaved as expected in both scenarios (${report.scenarios.reduce((n, sc) => n + sc.aggregate.length, 0)}/${report.scenarios.reduce((n, sc) => n + sc.aggregate.length, 0)} checks)`
      : ` SELF-TEST: FAIL — ${report.expectedVsActual.mismatches.join(" | ")}`
  );
  L.push(bar);
  return L.join("\n");
}

export { fmt, triple };
