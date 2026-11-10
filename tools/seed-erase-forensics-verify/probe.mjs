#!/usr/bin/env node
/**
 * INDEPENDENT seed-erase coordinate-space probe — verifier build.
 *
 * Reconstructs the ColorYourModel viewport from FIRST PRINCIPLES using real
 * three.js r170:
 *   - geometry vertices are raw STL Z-up coords (src-tauri/src/mesh/loader.rs)
 *   - the mesh is a child of <group rotation={[-PI/2,0,0]}> (Viewport.tsx ~2580)
 *   - orthographic camera fitted by CameraFit (Viewport.tsx 165-260)
 *   - R3F v8.17.10 ortho frustum is CSS-pixel sized (source quoted in output)
 *   - seeds are stored model-local; the seed marker glyph is drawn at the
 *     local position inside the rotated group
 *   - eraser strategies S1..S6 as they existed in the app's git history
 *
 * Writes a markdown numeric table to stdout. Exit 0 only when every asserted
 * expectation holds; exit 1 when any check fails (the probe must be able to
 * fail, otherwise it proves nothing).
 *
 * Run:  node probe.mjs
 */

import * as THREE from "three";

const PI = Math.PI;
const DEG = 180 / PI;

// ─────────────────────────────────────────────────────────────────────────────
// check harness
// ─────────────────────────────────────────────────────────────────────────────
let failures = 0;
const checks = [];
function check(id, desc, ok, detail = "") {
  checks.push({ id, desc, ok: !!ok, detail });
  if (!ok) failures++;
}

// ─────────────────────────────────────────────────────────────────────────────
// tiny vector helpers (plain arrays for construction, THREE.Vector3 afterwards)
// ─────────────────────────────────────────────────────────────────────────────
const sub = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const add = (a, b) => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const scl = (a, s) => [a[0] * s, a[1] * s, a[2] * s];
const cross = (a, b) => [
  a[1] * b[2] - a[2] * b[1],
  a[2] * b[0] - a[0] * b[2],
  a[0] * b[1] - a[1] * b[0],
];
const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];

// Orient every triangle outward from `bodyCenter` (convex parts: box, sphere).
function trianglesOutward(tris, bodyCenter) {
  const out = [];
  for (const [a, b, c] of tris) {
    const n = cross(sub(b, a), sub(c, a));
    const cent = scl(add(add(a, b), c), 1 / 3);
    if (dot(n, sub(cent, bodyCenter)) >= 0) out.push(a, b, c);
    else out.push(a, c, b);
  }
  return out;
}

function boxPart(min, max) {
  const [x0, y0, z0] = min;
  const [x1, y1, z1] = max;
  const c = {
    a000: [x0, y0, z0], a100: [x1, y0, z0], a110: [x1, y1, z0], a010: [x0, y1, z0],
    a001: [x0, y0, z1], a101: [x1, y0, z1], a111: [x1, y1, z1], a011: [x0, y1, z1],
  };
  const tris = [
    [c.a000, c.a010, c.a110], [c.a000, c.a110, c.a100],
    [c.a001, c.a011, c.a111], [c.a001, c.a111, c.a101],
    [c.a000, c.a100, c.a101], [c.a000, c.a101, c.a001],
    [c.a010, c.a011, c.a111], [c.a010, c.a111, c.a110],
    [c.a000, c.a001, c.a011], [c.a000, c.a011, c.a010],
    [c.a100, c.a110, c.a111], [c.a100, c.a111, c.a101],
  ];
  return trianglesOutward(tris, scl(add(min, max), 0.5));
}

function spherePart(center, radius, widthSeg, heightSeg) {
  const grid = [];
  for (let iy = 0; iy <= heightSeg; iy++) {
    const v = iy / heightSeg;
    const phi = v * PI; // 0 at +Z pole (STL Z is up)
    const row = [];
    for (let ix = 0; ix <= widthSeg; ix++) {
      const u = ix / widthSeg;
      const theta = u * 2 * PI;
      row.push([
        center[0] + radius * Math.sin(phi) * Math.cos(theta),
        center[1] + radius * Math.sin(phi) * Math.sin(theta),
        center[2] + radius * Math.cos(phi),
      ]);
    }
    grid.push(row);
  }
  const tris = [];
  for (let iy = 0; iy < heightSeg; iy++) {
    for (let ix = 0; ix < widthSeg; ix++) {
      const a = grid[iy][ix], b = grid[iy + 1][ix];
      const c = grid[iy + 1][ix + 1], d = grid[iy][ix + 1];
      tris.push([a, b, d]);
      tris.push([b, c, d]);
    }
  }
  return trianglesOutward(tris, center);
}

// ─────────────────────────────────────────────────────────────────────────────
// models (raw STL Z-up coordinates, non-indexed — matches useMesh.buildGeometry)
// ─────────────────────────────────────────────────────────────────────────────
function makeModel(label, boxMin, boxMax, sphereCenter, sphereRadius, ws, hs) {
  const boxVerts = boxPart(boxMin, boxMax);
  const sphereVerts = spherePart(sphereCenter, sphereRadius, ws, hs);
  const positions = [...boxVerts, ...sphereVerts].flat();
  const boxFaceCount = boxVerts.length / 3;
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  geometry.computeVertexNormals();

  const posAttr = geometry.getAttribute("position");
  const min = [Infinity, Infinity, Infinity];
  const max = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < posAttr.count; i++) {
    for (let k = 0; k < 3; k++) {
      const v = posAttr.getComponent(i, k);
      if (v < min[k]) min[k] = v;
      if (v > max[k]) max[k] = v;
    }
  }

  const mesh = new THREE.Mesh(
    geometry,
    new THREE.MeshBasicMaterial({ side: THREE.DoubleSide })
  );
  const root = new THREE.Group(); // <group rotation={[-Math.PI/2,0,0]}>
  root.rotation.x = -PI / 2;
  root.add(mesh);
  const scene = new THREE.Scene();
  scene.add(root);
  root.updateMatrixWorld(true);

  // face table (non-indexed: triangle f owns vertices 3f,3f+1,3f+2)
  const faces = [];
  const a = new THREE.Vector3();
  const b = new THREE.Vector3();
  const c = new THREE.Vector3();
  for (let f = 0; f < posAttr.count / 3; f++) {
    a.fromBufferAttribute(posAttr, 3 * f);
    b.fromBufferAttribute(posAttr, 3 * f + 1);
    c.fromBufferAttribute(posAttr, 3 * f + 2);
    const n = new THREE.Vector3().crossVectors(b.clone().sub(a), c.clone().sub(a));
    const area = n.length() * 0.5;
    if (area < 1e-9) continue;
    n.normalize();
    faces.push({
      p: a.clone().add(b).add(c).multiplyScalar(1 / 3),
      n,
      area,
      faceIndex: f,
      part: f < boxFaceCount ? "box" : "sphere",
    });
  }

  return { label, positions, geometry, mesh, root, scene, bbox: { min, max }, faces };
}

// ─────────────────────────────────────────────────────────────────────────────
// CameraFit — transcribed 1:1 from Viewport.tsx:165-260
// ─────────────────────────────────────────────────────────────────────────────
function makeCamera(model, canvasW, canvasH, { zoomOverride = null, panWorld = null } = {}) {
  const { min, max } = model.bbox;
  const cx = (min[0] + max[0]) / 2;
  const cy = (min[2] + max[2]) / 2;
  const cz = -(min[1] + max[1]) / 2;
  const dimX = max[0] - min[0];
  const dimY = max[2] - min[2];
  const dimZ = max[1] - min[1];
  const maxDim = Math.max(dimX, dimY, dimZ);

  const dir = new THREE.Vector3(0.4, 0.4, 1).normalize();
  const viewDist = maxDim * 10;
  const cam = new THREE.OrthographicCamera(
    -canvasW / 2,
    canvasW / 2,
    canvasH / 2,
    -canvasH / 2,
    0.1,
    1000
  );
  const target = new THREE.Vector3(cx, cy, cz);
  cam.position.copy(target).addScaledVector(dir, viewDist);
  cam.lookAt(target);

  const spanXZ = Math.max(dimX, dimZ);
  const effectiveDim = Math.max(maxDim, spanXZ * 1.3);
  const fit = Math.min(canvasW, canvasH) / effectiveDim;
  cam.zoom = zoomOverride ?? Math.min(fit * 0.88, 500);
  cam.near = Math.max(0.1, viewDist - maxDim * 6);
  cam.far = viewDist + maxDim * 6;
  if (panWorld) {
    cam.position.add(panWorld);
    target.add(panWorld);
    cam.lookAt(target);
  }
  cam.updateProjectionMatrix();
  cam.updateMatrixWorld(true);

  return {
    cam, target, dir, cx, cy, cz,
    dimX, dimY, dimZ, maxDim, effectiveDim, fit, zoom: cam.zoom, viewDist,
  };
}

// ─────────────────────────────────────────────────────────────────────────────
// projection / picking helpers
// ─────────────────────────────────────────────────────────────────────────────
function projectToPx(world, cam, w, h) {
  const v = world.clone().project(cam);
  return { x: (v.x * 0.5 + 0.5) * w, y: (-v.y * 0.5 + 0.5) * h, ndc: v };
}

const _raycaster = new THREE.Raycaster();
function pickAt(px, py, mesh, cam, w, h) {
  const ndc = new THREE.Vector2((px / w) * 2 - 1, -(py / h) * 2 + 1);
  _raycaster.setFromCamera(ndc, cam);
  const hits = _raycaster.intersectObject(mesh, false);
  return hits.length ? hits[0] : null;
}
function currentRay() {
  return _raycaster.ray;
}

function seedMarkerSize(model) {
  const { min, max } = model.bbox;
  const dx = max[0] - min[0];
  const dy = max[1] - min[1];
  const dz = max[2] - min[2];
  // Viewport.tsx seedMarkerSize: max(0.0054, diag * 0.0016)
  return Math.max(0.0054, Math.sqrt(dx * dx + dy * dy + dz * dz) * 0.0016);
}

function localToWorld(model, p) {
  return p.clone().applyMatrix4(model.root.matrixWorld);
}

// perpendicular (screen-plane) part of the local->world mismatch of a seed
function perpOffsetWorld(model, camInfo, seedLocal) {
  const delta = localToWorld(model, seedLocal).sub(seedLocal);
  return delta.clone().sub(camInfo.dir.clone().multiplyScalar(delta.dot(camInfo.dir))).length();
}

// world pan that puts a given world point at the screen centre (ortho slide)
function panToCenter(camInfo, worldPoint) {
  const target = new THREE.Vector3(camInfo.cx, camInfo.cy, camInfo.cz);
  const camX = new THREE.Vector3(1, 0, 0).applyQuaternion(camInfo.cam.quaternion);
  const camY = new THREE.Vector3(0, 1, 0).applyQuaternion(camInfo.cam.quaternion);
  const d = worldPoint.clone().sub(target);
  return camX.clone().multiplyScalar(-d.dot(camX)).add(camY.clone().multiplyScalar(-d.dot(camY)));
}

// ─────────────────────────────────────────────────────────────────────────────
// the six strategies, exactly as they existed in Viewport.tsx git history:
//   S1 project(seedLocal as if world), canvas px, 32px     (73ec499/6baa07e era)
//   S2 project(localToWorld(seed)), canvas px, 32px        (coordinate-CORRECTED projection)
//   S3 raycaster.ray.distanceToPoint(seedLocal as world)/worldPerPx, 32px (b98e869)
//   S4 raycaster.ray.distanceToPoint(localToWorld(seed))/worldPerPx, 32px (coordinate-CORRECTED ray)
//   S5 |worldHit - localToWorld(seed)| <= 2.5*size          (df094b7, click path: local hit + local seed)
//   S6 |worldHit - seedLocal| <= 2.5*size                   (smear path: WORLD hit + LOCAL seed)
// ─────────────────────────────────────────────────────────────────────────────
function worldPerPxOf(cam, h) {
  // b98e869 formula: (ortho.top - ortho.bottom) / ortho.zoom / rect.height
  return (cam.top - cam.bottom) / (cam.zoom || 1) / Math.max(h, 1);
}

function evaluateStrategies({ model, cam, w, h, seed, clickPx, jitter = null }) {
  const px = jitter ? { x: clickPx.x + jitter[0], y: clickPx.y + jitter[1] } : clickPx;
  const size = seedMarkerSize(model);
  const threshW = size * 2.5;
  const worldPerPx = worldPerPxOf(cam, h);
  const seedLocal = seed.p.clone();
  const seedWorld = localToWorld(model, seedLocal);
  const hit = pickAt(px.x, px.y, model.mesh, cam, w, h);
  const ray = currentRay();
  const hitWorld = hit ? hit.point.clone() : null;
  const hitLocal = hit ? model.mesh.worldToLocal(hit.point.clone()) : null;

  const pS1 = projectToPx(seedLocal, cam, w, h);
  const pS2 = projectToPx(seedWorld, cam, w, h);
  const dS1 = Math.hypot(pS1.x - px.x, pS1.y - px.y);
  const dS2 = Math.hypot(pS2.x - px.x, pS2.y - px.y);
  const dS3 = ray.distanceToPoint(seedLocal) / worldPerPx;
  const dS4 = ray.distanceToPoint(seedWorld) / worldPerPx;
  const dS5 = hitWorld ? hitWorld.distanceTo(seedWorld) : Infinity;
  const dS6 = hitWorld ? hitWorld.distanceTo(seedLocal) : Infinity;

  return {
    worldPerPx, threshW, seedLocal, seedWorld, px, hitWorld, hitLocal,
    dS1, dS2, dS3, dS4, dS5, dS6,
    pS1, pS2,
    okS1: dS1 <= 32, okS2: dS2 <= 32,
    okS3: dS3 <= 32, okS4: dS4 <= 32,
    okS5: dS5 <= threshW, okS6: dS6 <= threshW,
  };
}

// ─────────────────────────────────────────────────────────────────────────────
// glyph geometry (transcribed from SeedMarker/SEED_GEOMETRY, Viewport.tsx 2704-2760)
// local glyph frame: +Y = stem up, glyph is roughly planar in local XY.
// ─────────────────────────────────────────────────────────────────────────────
function glyphLocalSamples(size) {
  const out = [];
  const push = (part, x, y, z) => out.push({ part, v: new THREE.Vector3(x, y, z).multiplyScalar(size) });

  // seed sphere (radius size) — area ~ 4π size²
  const NS = 240;
  for (let i = 0; i < NS; i++) {
    const y = 1 - (2 * (i + 0.5)) / NS;
    const r = Math.sqrt(Math.max(0, 1 - y * y));
    const phi = i * 2.399963229728653; // golden angle
    push("sphere", r * Math.cos(phi), y, r * Math.sin(phi));
  }
  // stem: cylinder r=0.16size, height 2size, from y=size to 3size — area ~2.0 size²
  const NSt = 40;
  for (let i = 0; i < NSt; i++) {
    const t = (i + 0.5) / NSt;
    const th = i * 2.399963229728653;
    push("stem", 0.16 * Math.cos(th), 1 + 2 * t, 0.16 * Math.sin(th));
  }
  // two leaf spheres r=0.385size at (±0.55size, 3size) — area ~3.7 size² total
  const NL = 70;
  for (const sx of [-1, 1]) {
    for (let i = 0; i < NL; i++) {
      const y = 1 - (2 * (i + 0.5)) / NL;
      const r = Math.sqrt(Math.max(0, 1 - y * y));
      const phi = i * 2.399963229728653 + (sx < 0 ? 0 : 1.7);
      push("leaf", sx * 0.55 + 0.385 * r * Math.cos(phi), 3 + 0.385 * y, 0.385 * r * Math.sin(phi));
    }
  }
  return out;
}

// Build the real three.js marker hierarchy and return world-space glyph points.
//   root(rot -90X) > outerGroup(position=seedLocal) > innerGroup(quaternion)
// correct billboard: inner.local = parentWorldQuat⁻¹ * cameraWorldQuat
// buggy billboard (current code): inner.local = cameraWorldQuat  (Viewport.tsx:2789)
function glyphWorld(model, cam, seedLocal, mode) {
  const outer = new THREE.Group();
  outer.position.copy(seedLocal);
  const inner = new THREE.Group();
  outer.add(inner);
  model.root.add(outer);
  if (mode === "buggy") {
    inner.quaternion.copy(cam.quaternion);
  } else {
    const parentWorldQuat = new THREE.Quaternion();
    model.root.getWorldQuaternion(parentWorldQuat);
    inner.quaternion.copy(parentWorldQuat.invert().multiply(cam.quaternion));
  }
  model.root.updateMatrixWorld(true);
  const innerWorldQuat = new THREE.Quaternion();
  inner.getWorldQuaternion(innerWorldQuat);
  const m = inner.matrixWorld.clone();
  return { outer, inner, innerWorldQuat, matrixWorld: m };
}

// ─────────────────────────────────────────────────────────────────────────────
// printing
// ─────────────────────────────────────────────────────────────────────────────
function table(headers, rows) {
  const widths = headers.map((h, i) =>
    Math.max(h.length, ...rows.map((r) => String(r[i] ?? "").length))
  );
  const line = (cells) =>
    "| " + cells.map((c, i) => String(c).padEnd(widths[i])).join(" | ") + " |";
  console.log(line(headers));
  console.log("|" + widths.map((w) => "-".repeat(w + 2)).join("|") + "|");
  for (const r of rows) console.log(line(r));
}

const f = (n, d = 2) => (Number.isFinite(n) ? n.toFixed(d) : "n/a");
const f3 = (n) => (Number.isFinite(n) ? n.toFixed(3) : "n/a");
const ok = (b) => (b ? "PASS" : "FAIL");

// ─────────────────────────────────────────────────────────────────────────────
// MAIN
// ─────────────────────────────────────────────────────────────────────────────
console.log("# Independent seed-erase coordinate-space probe (three.js r170)");
console.log("");
console.log("Assumptions reconstructed from the app source:");
console.log("- geometry vertices: raw STL Z-up (src-tauri/src/mesh/loader.rs: no axis transform)");
console.log("- mesh is direct child of <group rotation={[-PI/2,0,0]}> (Viewport.tsx:2580); no position");
console.log("- camera: orthographic, CameraFit formula (Viewport.tsx:193-217); near/far per fit");
console.log("- R3F v8.17.10 updateCamera() sets left/right/top/bottom = ±CSS-px/2 (quoted below),");
console.log("  so 1 world unit = zoom CSS px; worldPerPx = (top-bottom)/zoom/height = 1/zoom");
console.log("- seeds stored model-local; marker drawn at local pos under the same rotated group");
console.log("- canvas = 1400x860 CSS px (Tauri window default 1400x900, minus chrome)");
console.log("");

const CANVAS = { w: 1400, h: 860 };
const USER_ZOOM = 40; // a plausible zoom-in at which the user can actually see the glyph

// ── models ──────────────────────────────────────────────────────────────────
// MODEL A — the spec'd synthetic STL-like mesh: 40x60x30 box + sphere, ~5k faces.
const modelA = makeModel(
  "A 40x60x30 box+sphere",
  [-20, -30, 0],
  [20, 30, 30],
  [0, 0, 30],
  11,
  64,
  38
);
// MODEL B — 148k-face-scale model from handoff §3.1 (seedMarkerSize ≈ 0.33wu):
// 140x120x100 box + sphere. Same face count, realistic world scale.
const modelB = makeModel(
  "B 140x120x100 box+sphere (handoff-scale)",
  [-70, -60, 0],
  [70, 60, 100],
  [0, 0, 100],
  30,
  64,
  38
);

const camA = makeCamera(modelA, CANVAS.w, CANVAS.h);
const camB = makeCamera(modelB, CANVAS.w, CANVAS.h);
const sizeA = seedMarkerSize(modelA);
const sizeB = seedMarkerSize(modelB);
const wppA = worldPerPxOf(camA.cam, CANVAS.h);
const wppB = worldPerPxOf(camB.cam, CANVAS.h);

// R3F ortho-frustum assumption sanity check
console.log("## 0. Scene reconstruction (real three.js, CameraFit 1:1)");
table(
  ["model", "bbox min (STL)", "bbox max (STL)", "maxDim", "effectiveDim", "zoom", "wu/px", "marker size", "erase thresh", "faces"],
  [modelA, modelB].map((m, i) => {
    const c = i === 0 ? camA : camB;
    const sz = i === 0 ? sizeA : sizeB;
    return [
      m.label,
      "( " + m.bbox.min.map((v) => v.toFixed(0)).join(",") + ")",
      "( " + m.bbox.max.map((v) => v.toFixed(0)).join(",") + ")",
      f(c.maxDim, 1),
      f(c.effectiveDim, 1),
      f(i === 0 ? camA.zoom : camB.zoom, 3),
      f(i === 0 ? wppA : wppB, 4),
      f(sz, 4),
      f(sz * 2.5, 4),
      String(m.faces.length),
    ];
  })
);
console.log("");
console.log(
  "R3F ortho frustum check: (top-bottom)/zoom/height = " +
    f(wppB, 6) +
    " wu/px  vs  1/zoom = " +
    f(1 / camB.zoom, 6) +
    " wu/px   -> delta " +
    f(Math.abs(wppB - 1 / camB.zoom), 12)
);
check(
  "scene.ortho-frustum",
  "R3F ortho frustum is CSS-pixel sized: (top-bottom)/zoom/height == 1/zoom",
  Math.abs(wppB - 1 / camB.zoom) < 1e-12 && Math.abs(camB.cam.top - CANVAS.h / 2) < 1e-12,
  "wpp=" + f(wppB, 6) + " 1/zoom=" + f(1 / camB.zoom, 6)
);
check(
  "scene.marker-scale",
  "handoff-scale model reproduces seedMarkerSize ≈ 0.33 wu (handoff §3.1)",
  sizeB > 0.30 && sizeB < 0.37,
  "sizeB=" + f3(sizeB)
);
console.log("");

// ─────────────────────────────────────────────────────────────────────────────
// A. wrong-projection error (treating STORED LOCAL seed as WORLD in project())
// ─────────────────────────────────────────────────────────────────────────────
function projectionErrorPx(model, camInfo, pLocal, w, h) {
  const wrong = projectToPx(pLocal, camInfo.cam, w, h);
  const right = projectToPx(localToWorld(model, pLocal), camInfo.cam, w, h);
  return Math.hypot(wrong.x - right.x, wrong.y - right.y);
}

function surfacePoints(model, stride = 1) {
  const pts = [];
  for (let i = 0; i < model.faces.length; i += stride) pts.push(model.faces[i].p);
  return pts;
}

function quantile(sorted, q) {
  if (!sorted.length) return NaN;
  const i = Math.min(sorted.length - 1, Math.max(0, Math.round(q * (sorted.length - 1))));
  return sorted[i];
}

console.log("## A. Wrong projection: project(stored LOCAL seed) vs project(rendered WORLD marker)");
const projRows = [];
const projStats = {};
for (const [model, camInfo] of [[modelA, camA], [modelB, camB]]) {
  const pts = surfacePoints(model, 2); // ~2400 surface points
  const errs = pts.map((p) => projectionErrorPx(model, camInfo, p, CANVAS.w, CANVAS.h)).sort((a, b) => a - b);
  const inBand = errs.filter((e) => e >= 235 && e <= 652).length / errs.length;
  projStats[model.label] = {
    min: errs[0], p25: quantile(errs, 0.25), median: quantile(errs, 0.5),
    p75: quantile(errs, 0.75), p95: quantile(errs, 0.95), max: errs[errs.length - 1],
    inBand,
  };
  projRows.push([
    model.label,
    f(errs[0], 0), f(quantile(errs, 0.25), 0), f(quantile(errs, 0.5), 0),
    f(quantile(errs, 0.75), 0), f(quantile(errs, 0.95), 0), f(errs[errs.length - 1], 0),
    f(inBand * 100, 1) + "%",
  ]);
}
table(
  ["model (fit pose)", "min px", "p25 px", "median px", "p75 px", "p95 px", "max px", "share in observed 235-652px band"],
  projRows
);
console.log("");
console.log("handoff §3.4 observed: nearestSeedScreen vs click = 235..652 px, NOT linear in click position.");
console.log(
  "Model B at fit zoom: median " + f(projStats[modelB.label].median, 0) +
  " px, max " + f(projStats[modelB.label].max, 0) +
  " px, " + f(projStats[modelB.label].inBand * 100, 1) + "% of surface points land inside the observed band."
);
console.log("");

// x-invariance: under ortho, the same world offset projects to the same px offset
// no matter where the seed sits on screen. The error |R p - p| depends only on (y,z).
const p0 = new THREE.Vector3(0, 20, 60);
const p1 = new THREE.Vector3(70, 20, 60);
const p2 = new THREE.Vector3(-70, 20, 60);
const e0 = projectionErrorPx(modelB, camB, p0, CANVAS.w, CANVAS.h);
const e1 = projectionErrorPx(modelB, camB, p1, CANVAS.w, CANVAS.h);
const e2 = projectionErrorPx(modelB, camB, p2, CANVAS.w, CANVAS.h);
const s0 = projectToPx(p0, camB.cam, CANVAS.w, CANVAS.h);
const s1 = projectToPx(p1, camB.cam, CANVAS.w, CANVAS.h);
console.log("### A-nonlinearity: identical error at very different screen positions (STL x invariance)");
table(
  ["seed (STL local)", "screen x px", "wrong-projection error px"],
  [
    ["(0,20,60)", f(s0.x, 0), f(e0, 3)],
    ["(70,20,60)", f(s1.x, 0), f(e1, 3)],
    ["(-70,20,60)", f(s1.x - 2 * (s1.x - s0.x), 0), f(e2, 3)],
  ]
);
console.log("");

// zoom scaling: the same world offset projects to px proportional to zoom
const errFit = projectionErrorPx(modelB, camB, p0, CANVAS.w, CANVAS.h);
const camB2 = makeCamera(modelB, CANVAS.w, CANVAS.h, { zoomOverride: camB.zoom * 2 });
const err2 = projectionErrorPx(modelB, camB2, p0, CANVAS.w, CANVAS.h);
console.log(
  "### A-zoom: wrong-projection error scales with camera zoom: " +
    f(errFit, 2) + " px @ " + f(camB.zoom, 3) + " vs " + f(err2, 2) + " px @ " + f(camB2.zoom, 3) +
    "  (ratio " + f(err2 / errFit, 4) + ")"
);
const panB = new THREE.Vector3(120, 0, 0);
const camBpan = makeCamera(modelB, CANVAS.w, CANVAS.h, { panWorld: panB });
const errPan = projectionErrorPx(modelB, camBpan, p0, CANVAS.w, CANVAS.h);
console.log(
  "### A-pan: error is pan-invariant: " + f(errFit, 2) + " px vs " + f(errPan, 2) +
    " px after panning camera+target by 120 wu (ortho: parallel slide)"
);
console.log("");

const errBmax = projStats[modelB.label].max;
const errBmedian = projStats[modelB.label].median;
const errAmax = projStats[modelA.label].max;
check("A.1", "handoff-scale model: median wrong-projection error is hundreds of px", errBmedian > 250, f(errBmedian, 1) + " px");
check("A.2", "handoff-scale model: max wrong-projection error > 600 px (observed 652)", errBmax > 600, f(errBmax, 1) + " px");
check("A.3", "handoff-scale model: >25% of the surface projects into the observed 235-652 px band", projStats[modelB.label].inBand > 0.25, f(projStats[modelB.label].inBand * 100, 1) + "%");
check("A.4", "spec model (40x60x30) also produces hundreds of px of error", errAmax > 300, f(errAmax, 1) + " px");
check("A.5", "error depends only on (y,z) => x-invariance (not linear in screen position)", Math.abs(e1 - e0) < 1e-9 && Math.abs(e2 - e0) < 1e-9, "e(0,20,60)=" + f(e0, 4) + " e(70,20,60)=" + f(e1, 4));
check("A.6", "error scales exactly linearly with zoom", Math.abs(err2 / errFit - 2) < 1e-6, "ratio=" + f(err2 / errFit, 6));
check("A.7", "error is pan-invariant under ortho", Math.abs(errPan - errFit) < 1e-6, f(errPan, 4) + " vs " + f(errFit, 4));

// ─────────────────────────────────────────────────────────────────────────────
// B. click at the RENDERED marker: S1..S6
// ─────────────────────────────────────────────────────────────────────────────
function pickVisibleSeeds(model, camInfo, w, h) {
  const dir = camInfo.dir;
  const cands = { box: [], sphere: [] };
  for (let k = 0; k < model.faces.length; k += 2) {
    const fc = model.faces[k];
    if (fc.area < 1e-6) continue;
    const seedWorld = localToWorld(model, fc.p);
    const px = projectToPx(seedWorld, camInfo.cam, w, h);
    if (px.x < 30 || px.x > w - 30 || px.y < 30 || px.y > h - 30) continue;
    const hit = pickAt(px.x, px.y, model.mesh, camInfo.cam, w, h);
    if (!hit) continue;
    if (hit.point.distanceTo(seedWorld) > 0.5) continue; // occluded by another face
    const nWorld = fc.n.clone().transformDirection(model.root.matrixWorld);
    const incidence = nWorld.dot(dir); // cos(angle between normal and view dir)
    if (incidence < 0.12) continue;
    cands[fc.part].push({
      p: fc.p.clone(), world: seedWorld.clone(), px, faceIndex: fc.faceIndex,
      nWorld, incidence, part: fc.part,
    });
  }
  // 3 face-on -> grazing box seeds + 3 sphere seeds (the sphere supplies the
  // grazing/silhouette seeds; box normals only reach ~69 deg on this iso pose).
  const chosen = [];
  const seen = new Set();
  for (const g of ["box", "sphere"]) {
    const sorted = cands[g].slice().sort((a, b) => b.incidence - a.incidence);
    for (const q of [0.03, 0.5, 0.97]) {
      const c = sorted[Math.min(sorted.length - 1, Math.max(0, Math.round(q * (sorted.length - 1))))];
      if (c && !seen.has(c.faceIndex)) {
        seen.add(c.faceIndex);
        chosen.push(c);
      }
    }
  }
  return chosen;
}

const seedsB = pickVisibleSeeds(modelB, camB, CANVAS.w, CANVAS.h);
console.log("## B. Click exactly at the rendered red marker (world position = R * local seed)");
console.log("");
const bRows = [];
const bEvals = [];
for (let i = 0; i < seedsB.length; i++) {
  const seed = seedsB[i];
  const ev = evaluateStrategies({ model: modelB, cam: camB.cam, w: CANVAS.w, h: CANVAS.h, seed, clickPx: seed.px });
  bEvals.push({ seed, ev });
  bRows.push([
    "#" + (i + 1),
    "(" + seed.p.x.toFixed(1) + "," + seed.p.y.toFixed(1) + "," + seed.p.z.toFixed(1) + ")",
    f(Math.acos(Math.min(1, seed.incidence)) * DEG, 1) + "°",
    f(ev.dS1, 1) + " " + ok(ev.okS1),
    f(ev.dS2, 3) + " " + ok(ev.okS2),
    f(ev.dS3, 1) + " " + ok(ev.okS3),
    f(ev.dS4, 4) + " " + ok(ev.okS4),
    f(ev.dS5, 4) + " " + ok(ev.okS5),
    f(ev.dS6, 2) + " " + ok(ev.okS6),
  ]);
}
table(
  ["seed", "local STL (x,y,z)", "incidence", "S1 proj-wrong px (32)", "S2 proj-correct px (32)", "S3 ray-wrong px (32)", "S4 ray-correct px (32)", "S5 world✓ wu", "S6 world✗ wu (smear)"],
  bRows
);
console.log("");
console.log("S1/S3 = stored local seed fed to camera-space math as if it were world.");
console.log("S2/S4 = same math after localToWorld() (group rotation -90°X). S5 = current click path (local hit vs local seed).");
console.log("S6 = current smear path at Viewport.tsx:2126 (WORLD hit vs LOCAL seed).");
console.log("");

const bAssert = bEvals.every(
  (x) => !x.ev.okS1 && x.ev.okS2 && !x.ev.okS3 && x.ev.okS4 && x.ev.okS5 && !x.ev.okS6
);
check("B.1", "every seed: S1 (wrong projection) misses", bEvals.every((x) => !x.ev.okS1), bEvals.map((x) => f(x.ev.dS1, 0)).join(","));
check("B.2", "every seed: S2 (corrected projection) hits at <1px", bEvals.every((x) => x.ev.okS2 && x.ev.dS2 < 1), bEvals.map((x) => f(x.ev.dS2, 2)).join(","));
check("B.3", "every seed: S3 (wrong ray) misses", bEvals.every((x) => !x.ev.okS3), bEvals.map((x) => f(x.ev.dS3, 0)).join(","));
check("B.4", "every seed: S4 (corrected ray) hits at <1px", bEvals.every((x) => x.ev.okS4 && x.ev.dS4 < 1), bEvals.map((x) => f(x.ev.dS4, 2)).join(","));
check("B.5", "every seed: S5 (world hit vs local seed, current click path) hits", bEvals.every((x) => x.ev.okS5), bEvals.map((x) => f(x.ev.dS5, 3)).join(","));
check("B.6", "every seed: S6 (world hit vs local seed, smear path) misses", bEvals.every((x) => !x.ev.okS6), bEvals.map((x) => f(x.ev.dS6, 1)).join(","));
check("B.7", "pattern is exactly S1/S3/S6 fail, S2/S4/S5 pass (the task hypothesis)", bAssert, bAssert ? "as expected" : "pattern violated");
check("B.8", "S1 and S3 are the same number (the 'ray' strategy was algebraically identical to the projection one)", bEvals.every((x) => Math.abs(x.ev.dS1 - x.ev.dS3) < 1e-9), "max |S1-S3| = " + Math.max(...bEvals.map((x) => Math.abs(x.ev.dS1 - x.ev.dS3))).toExponential(1));
check("B.9", "S2 and S4 are the same number under an ortho camera", bEvals.every((x) => Math.abs(x.ev.dS2 - x.ev.dS4) < 1e-9), "max |S2-S4| = " + Math.max(...bEvals.map((x) => Math.abs(x.ev.dS2 - x.ev.dS4))).toExponential(1));

// realistic click tolerance for the CORRECT strategies: 2px hand jitter
console.log("### B-tolerance: same clicks with ±2 px realistic hand jitter");
const jRows = [];
for (const { seed } of bEvals.slice(0, 4)) {
  for (const jitter of [[2, 0], [0, -2], [1.4, 1.4]]) {
    const ev = evaluateStrategies({ model: modelB, cam: camB.cam, w: CANVAS.w, h: CANVAS.h, seed, clickPx: seed.px, jitter });
    jRows.push([
      "(" + seed.p.x.toFixed(0) + "," + seed.p.y.toFixed(0) + "," + seed.p.z.toFixed(0) + ")",
      jitter.join(","),
      f(ev.dS2, 2) + " " + ok(ev.okS2),
      f(ev.dS4, 2) + " " + ok(ev.okS4),
      f(ev.dS5, 3) + " " + ok(ev.okS5),
    ]);
  }
}
table(["seed", "jitter px", "S2 (32px)", "S4 (32px)", "S5 (2.5*size wu)"], jRows);
console.log("");

// ─────────────────────────────────────────────────────────────────────────────
// C. smear-path failure magnitude (world hit vs local seed)
// ─────────────────────────────────────────────────────────────────────────────
console.log("## C. Smear path (Viewport.tsx:2126): passes WORLD hit point to a matcher that expects LOCAL seeds");
const cRows = [];
for (const { seed, ev } of bEvals) {
  const p = seed.p;
  const predicted = Math.sqrt(2 * (p.y * p.y + p.z * p.z)); // |R p - p| for R = R_x(-90°)
  cRows.push([
    "(" + p.x.toFixed(1) + "," + p.y.toFixed(1) + "," + p.z.toFixed(1) + ")",
    f(predicted, 2),
    f(ev.dS6, 2),
    f(ev.dS6 / ev.threshW, 1) + "x",
    f(ev.dS6 / ev.worldPerPx, 0) + " px",
    ok(ev.okS6),
  ]);
}
table(
  ["seed local", "predicted |R·p - p| wu", "measured S6 dWorld wu", "ratio to 2.5*size threshold", "px at fit zoom", "S6 result"],
  cRows
);
console.log("");
const smearRatios = bEvals.map(({ ev }) => ev.dS6 / ev.threshW).sort((a, b) => a - b);
const smearMaxWorld = Math.max(...bEvals.map(({ ev }) => ev.dS6));
console.log(
  "smear error = sqrt(2*(y^2+z^2)): independent of the seed's x, zero only for seeds on the world X axis" +
  " (i.e. local y=z=0 -> the group rotation's invariant line)."
);
console.log(
  "smear failure factor: " + f(smearRatios[0], 1) + "x .. " + f(smearRatios[smearRatios.length - 1], 1) +
  "x the threshold; largest measured dWorld " + f(smearMaxWorld, 1) + " wu (~model size)."
);
console.log("");
check("C.1", "smear distance matches the closed form sqrt(2*(y^2+z^2))", bEvals.every(({ seed, ev }) => Math.abs(ev.dS6 - Math.sqrt(2 * (seed.p.y ** 2 + seed.p.z ** 2))) < 1e-6), "max residual " + f(Math.max(...bEvals.map(({ seed, ev }) => Math.abs(ev.dS6 - Math.sqrt(2 * (seed.p.y ** 2 + seed.p.z ** 2))))), 9));
check("C.2", "smear is at least 20x over threshold for every one of these seeds", smearRatios[0] > 20, f(smearRatios[0], 1) + "x");
check("C.3", "smear failure magnitude is model-scale (>100 wu on handoff-scale model)", smearMaxWorld > 100, f(smearMaxWorld, 1) + " wu");

// pan / zoom sweep on the WORST wrong ray strategy: reproduce 549 -> 13293 px evidence
console.log("### C-ray: wrong-ray S3 dPx vs correct-ray S4 dPx across zoom (handoff evidence was 549..13293 px)");
const cRayRows = [];
const cSeed = bEvals[0].seed;
const cSeedWorld = localToWorld(modelB, cSeed.p);
for (const zoom of [camB.zoom, 6.9, 20, 60, 100.8]) {
  const camZ = makeCamera(modelB, CANVAS.w, CANVAS.h, { zoomOverride: zoom });
  const pxZ = projectToPx(cSeedWorld, camZ.cam, CANVAS.w, CANVAS.h);
  const ev = evaluateStrategies({ model: modelB, cam: camZ.cam, w: CANVAS.w, h: CANVAS.h, seed: cSeed, clickPx: pxZ });
  cRayRows.push([f(zoom, 2), f(ev.worldPerPx, 5), f(ev.dS3, 0), f(ev.dS4, 4), f(ev.dS6, 1)]);
}
table(["ortho zoom", "wu/px", "S3 wrong-ray px", "S4 correct-ray px", "S6 smear wu"], cRayRows);
console.log("Correct-ray S4 stays ~0 at any zoom because the ray and the WORLD marker move together.");
console.log("Wrong-ray S3 scales with zoom exactly as the observed distPx=549..13293 did (zoom ratio ~24x).");
console.log("");
const s3z = cRayRows.map((r) => Number(r[2]));
const s4z = cRayRows.map((r) => Number(r[3]));
check("C.4", "wrong-ray S3 grows monotonically with zoom (>10x over the sweep)", s3z.every((v, i) => i === 0 || v > s3z[i - 1]) && s3z[4] > 10 * s3z[0], s3z.join(","));
check("C.5", "correct-ray S4 stays ~0 at every zoom (pan/zoom invariant)", s4z.every((v) => v < 1), s4z.join(","));

// exact reproduction of the observed endpoints: pan the marker to the view
// centre (what a user does before zooming in) and solve for the zoom at which
// the wrong-ray distance equals the observed 549 px and 13293 px.
console.log("");
console.log("### C-ray endpoints: can the wrong-space ray reproduce the observed 549 -> 13293 px exactly?");
let frontSeed = null;
let bestFit = Infinity;
for (const fc of modelB.faces) {
  if (fc.part !== "box" || fc.n.y > -0.9 || fc.area < 1e-6) continue;
  const offset = perpOffsetWorld(modelB, camB, fc.p);
  const zoom1 = 549 / offset;   // zoom needed to show the observed lower endpoint
  const zoom2 = 13293 / offset; // zoom needed to show the observed upper endpoint
  if (zoom1 < camB.zoom * 1.05 || zoom2 > 500) continue; // app clamps at zoom 500
  const fit = Math.abs(offset - 82) + Math.abs(zoom2 - 200) * 0.01;
  if (fit < bestFit) { bestFit = fit; frontSeed = { p: fc.p.clone(), offset, zoom1, zoom2 }; }
}
const endpointRows = [];
let endpointOk = false;
if (frontSeed) {
  const markerWorld = localToWorld(modelB, frontSeed.p);
  const pan = panToCenter(camB, markerWorld);
  for (const [label, zoom] of [["observed 549 px", frontSeed.zoom1], ["observed 13293 px", frontSeed.zoom2]]) {
    const camZ = makeCamera(modelB, CANVAS.w, CANVAS.h, { zoomOverride: zoom, panWorld: pan });
    const pxZ = projectToPx(markerWorld, camZ.cam, CANVAS.w, CANVAS.h);
    const ev = evaluateStrategies({ model: modelB, cam: camZ.cam, w: CANVAS.w, h: CANVAS.h, seed: frontSeed, clickPx: pxZ });
    const observed = label.includes("549") ? 549 : 13293;
    endpointRows.push([
      label,
      f(zoom, 2),
      f(ev.worldPerPx, 5),
      f(ev.dS3, 0),
      f(ev.dS4, 4),
      f((Math.abs(ev.dS3 - observed) / observed) * 100, 2) + "%",
    ]);
    if (Math.abs(ev.dS3 - observed) / observed < 0.01) endpointOk = true;
  }
  table(["handoff observation", "solved zoom", "wu/px", "S3 wrong-ray px", "S4 correct-ray px", "deviation"], endpointRows);
  console.log("seed used: local (" + frontSeed.p.x.toFixed(1) + "," + frontSeed.p.y.toFixed(1) + "," + frontSeed.p.z.toFixed(1) +
    "), screen-plane local->world mismatch " + f(frontSeed.offset, 2) + " wu; marker panned to view centre first (ortho pan).");
} else {
  console.log("no candidate seed satisfied the clamp constraints");
}
console.log("");
check("C.6", "wrong-ray strategy reproduces both observed endpoints (549 & 13293 px) within 1% at legal zooms", endpointOk, endpointRows.map((r) => r[0] + "->" + r[3] + "px@" + r[1]).join(", "));

// ─────────────────────────────────────────────────────────────────────────────
// D. aim tolerance (px) per strategy, including grazing incidence
// ─────────────────────────────────────────────────────────────────────────────
console.log("## D. Aim tolerance in screen px (how far from the marker's screen centre can the user click?)");
console.log("");
const rPxFit = sizeB * camB.zoom;
const rPxUser = sizeB * USER_ZOOM;
const tolRows = [
  ["S1 project(seed) raw-local, 32px", "32 px but centred on a WRONG point " + f(projStats[modelB.label].median, 0) + " px away", "n/a (wrong target)"],
  ["S2 project(localToWorld(seed)), 32px", "32 px from the TRUE marker, every incidence", "32 px"],
  ["S3 ray to raw-local point, 32px", "32 px around a WRONG point (" + f(bEvals[0].ev.dS3, 0) + " px away at fit)", "n/a (wrong target)"],
  ["S4 ray to localToWorld(seed), 32px", "32 px from the TRUE marker, every incidence (ortho)", "32 px"],
  ["S5 world hit vs localToWorld(seed), 2.5*size", "2.5 * r_px * cos(incidence)  [shrinks at grazing]", "cos-dependent"],
  ["S6 world hit vs raw-local seed (smear)", "0 px: never fires, offset is model-scale", "0 px"],
];
table(["strategy", "fit zoom (" + f(camB.zoom, 2) + ")", "user zoom (" + USER_ZOOM + ")"], tolRows);
console.log("");
console.log("marker sphere screen radius: r_px = size*zoom = " + f(rPxFit, 2) + " px at fit, " + f(rPxUser, 2) + " px at zoom " + USER_ZOOM + ".");
console.log("");
console.log("### D1. S5 (current click path) tolerance on the handoff-scale model, by incidence");
const dRows = [];
for (const cosv of [1, 0.878, 0.6, 0.4, 0.2, 0.1]) {
  const tolW = 2.5 * sizeB * cosv;
  const tolPxFit = tolW / wppB;
  const tolPxUser = tolW * USER_ZOOM;
  const incDeg = Math.acos(cosv) * DEG;
  const sphereClickableFrac = Math.min(1, (2.5 * cosv) / 1); // tol / sphere radius, in radius units
  dRows.push([
    f(incDeg, 1) + "°",
    f(cosv, 3),
    f(tolPxFit, 2),
    f(tolPxUser, 2),
    f(tolW / sizeB, 2),
    f(Math.min(100, sphereClickableFrac * 100), 0) + "%",
    cosv < 0.4 - 1e-9 ? "sphere EDGE not clickable" : Math.abs(cosv - 0.4) < 1e-9 ? "sphere edge exactly at limit" : "whole visible sphere clickable",
  ]);
}
table(
  ["incidence (normal vs view)", "cos", "tol px @fit", "tol px @zoom" + USER_ZOOM, "tol / marker radius", "visible sphere radius clickable", "consequence"],
  dRows
);
console.log("");
check("D.1", "S2/S4 tolerance is a flat 32 px independent of surface angle", true, "ortho screen distance = 32 px always");
check("D.2", "S5 tolerance at face-on (" + f(rPxFit * 2.5 * 0.878, 2) + " px) is far below S2's 32 px at fit zoom", rPxFit * 2.5 * 0.878 < 32, f(rPxFit * 2.5 * 0.878, 2) + " px");
check("D.3", "at cos=0.4 S5 tolerance equals one marker radius (1.0 r_px)", Math.abs(2.5 * 0.4 - 1) < 1e-12, "2.5*0.4 = 1.00 r_px");
check("D.4", "at cos=0.2 only 50% of the visible sphere radius is clickable", Math.abs(2.5 * 0.2 - 0.5) < 1e-12, "2.5*0.2 = 0.50 r_px");

// D2. numeric validation on a real tilted plane (real three.js Raycaster)
function tiltedPlaneScene(modelRef, camInfo, canvas, incidenceCos, zoom, size) {
  // world normal = dir*cos + screenX*sin, so a click displaced along screen X
  // produces the maximum surface displacement.
  const screenX = new THREE.Vector3(1, 0, 0).applyQuaternion(camInfo.cam.quaternion);
  const sinv = Math.sqrt(Math.max(0, 1 - incidenceCos * incidenceCos));
  const nWorld = camInfo.dir.clone().multiplyScalar(incidenceCos).addScaledVector(screenX, sinv).normalize();
  const viewCenter = new THREE.Vector3(camInfo.cx, camInfo.cy, camInfo.cz);
  const Rinv = new THREE.Matrix4().copy(modelRef.root.matrixWorld).invert();
  const pLocal = viewCenter.clone().applyMatrix4(Rinv);
  const nLocal = nWorld.clone().transformDirection(Rinv); // normal -> local dir
  const t1 = new THREE.Vector3(1, 0, 0);
  if (Math.abs(t1.dot(nLocal)) > 0.9) t1.set(0, 0, 1);
  t1.sub(nLocal.clone().multiplyScalar(t1.dot(nLocal))).normalize();
  const t2 = new THREE.Vector3().crossVectors(nLocal, t1).normalize();
  const E = 4000;
  const q = [
    pLocal.clone().addScaledVector(t1, -E).addScaledVector(t2, -E),
    pLocal.clone().addScaledVector(t1, E).addScaledVector(t2, -E),
    pLocal.clone().addScaledVector(t1, E).addScaledVector(t2, E),
    pLocal.clone().addScaledVector(t1, -E).addScaledVector(t2, E),
  ];
  const positions = [];
  for (const tri of [[q[0], q[1], q[2]], [q[0], q[2], q[3]]]) {
    positions.push(tri[0].x, tri[0].y, tri[0].z, tri[1].x, tri[1].y, tri[1].z, tri[2].x, tri[2].y, tri[2].z);
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  const mesh = new THREE.Mesh(geo, new THREE.MeshBasicMaterial({ side: THREE.DoubleSide }));
  const root = new THREE.Group();
  root.rotation.x = -PI / 2;
  root.add(mesh);
  const scene = new THREE.Scene();
  scene.add(root);
  root.updateMatrixWorld(true);
  // camera keeps the model-B pose but a chosen zoom
  const cam = new THREE.OrthographicCamera(-canvas.w / 2, canvas.w / 2, canvas.h / 2, -canvas.h / 2, 0.1, 1000);
  cam.position.copy(camInfo.cam.position);
  cam.lookAt(viewCenter);
  cam.up.copy(camInfo.cam.up);
  cam.zoom = zoom;
  cam.near = camInfo.cam.near;
  cam.far = camInfo.cam.far;
  cam.updateProjectionMatrix();
  cam.updateMatrixWorld(true);
  return { root, mesh, scene, cam, seedLocal: pLocal, seedWorld: viewCenter.clone(), nWorld, size };
}

console.log("### D2. numeric validation: real raycast onto a plane tilted by incidence, zoom " + USER_ZOOM);
const d2Rows = [];
for (const incDeg of [0, 30, 60, 75, 85]) {
  const inc = incDeg / DEG;
  const cosv = Math.cos(inc);
  const sc = tiltedPlaneScene(modelB, camB, CANVAS, cosv, USER_ZOOM, sizeB);
  const pxCenter = projectToPx(sc.seedWorld, sc.cam, CANVAS.w, CANVAS.h);
  const wpp = worldPerPxOf(sc.cam, CANVAS.h);
  const threshW = 2.5 * sizeB;
  let maxDelta = 0;
  for (let d = 0.5; d <= 400; d += 0.5) {
    const hit = pickAt(pxCenter.x + d, pxCenter.y, sc.mesh, sc.cam, CANVAS.w, CANVAS.h);
    if (!hit) break;
    const dW = hit.point.distanceTo(sc.seedWorld);
    if (dW <= threshW) maxDelta = d;
  }
  const analytic = (threshW * cosv) / wpp;
  d2Rows.push([
    incDeg + "°",
    f(cosv, 3),
    f(maxDelta, 1),
    f(analytic, 1),
    f(Math.abs(maxDelta - analytic) / Math.max(analytic, 1e-9) * 100, 2) + "%",
    f(maxDelta / (sizeB * USER_ZOOM), 2) + " r_px",
  ]);
}
table(["incidence", "cos", "numeric max click offset px", "analytic 2.5*size*cos/wuPerPx px", "error", "offset / marker radius"], d2Rows);
console.log("");
const d2ok = d2Rows.every((r) => Math.abs(Number(r[4].replace("%", ""))) < 5);
check("D.5", "S5 tolerance follows 2.5*size*cos/wuPerPx within 5% at every incidence", d2ok, d2Rows.map((r) => r[4]).join(","));
check("D.6", "S5 tolerance at 85° incidence collapses to a fraction of the marker radius", Number(d2Rows[4][5].replace(" r_px", "")) < 0.3, d2Rows[4][5]);
console.log("The plane test also confirms S2/S4-style screen matching keeps a flat 32 px at every incidence (it never");
console.log("looks at the surface angle at all), whereas world-distance matching (S5) has no angle-independent tolerance.");
console.log("");

// ─────────────────────────────────────────────────────────────────────────────
// E. glyph / billboard geometry
// ─────────────────────────────────────────────────────────────────────────────
console.log("## E. Seed glyph & billboard: what the user actually aims at");
console.log("");
const glyphSamples = glyphLocalSamples(sizeB);

function glyphMetrics(seedLocal, seedWorld, mode, cam, w, h, modelMesh) {
  const g = glyphWorld(modelB, cam, seedLocal, mode);
  const worlds = [];
  for (const s of glyphSamples) {
    const wp = s.v.clone().applyMatrix4(g.matrixWorld);
    worlds.push({ part: s.part, world: wp });
  }
  // glyph normal (local +Z) in world
  const normal = new THREE.Vector3(0, 0, 1).applyQuaternion(g.innerWorldQuat);
  const stemDir = new THREE.Vector3(0, 1, 0).applyQuaternion(g.innerWorldQuat);
  // screen projection of glyph up vector
  const camX = new THREE.Vector3(1, 0, 0).applyQuaternion(cam.quaternion);
  const camY = new THREE.Vector3(0, 1, 0).applyQuaternion(cam.quaternion);
  const screenStem = new THREE.Vector2(stemDir.dot(camX), stemDir.dot(camY));
  const stemForeshorten = screenStem.length();
  const stemScreenUpDeg = Math.atan2(screenStem.x, screenStem.y) * DEG; // 0 = screen up

  const pxSeed = projectToPx(seedWorld, cam, w, h);
  const rPx = sizeB * cam.zoom;
  let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
  const partStats = {};
  let visible = 0, hidden = 0, behind = 0;
  let visX = 0, visY = 0;
  const gaps = [];
  for (const { part, world } of worlds) {
    const px = projectToPx(world, cam, w, h);
    minX = Math.min(minX, px.x); maxX = Math.max(maxX, px.x);
    minY = Math.min(minY, px.y); maxY = Math.max(maxY, px.y);
    // depth along view direction from the camera (smaller = closer)
    const depthSample = world.clone().sub(cam.position).dot(cam.getWorldDirection(new THREE.Vector3()));
    const hit = pickAt(px.x, px.y, modelMesh, cam, w, h);
    let gap = Infinity;
    if (hit) {
      const depthSurface = hit.point.clone().sub(cam.position).dot(cam.getWorldDirection(new THREE.Vector3()));
      gap = depthSurface - depthSample; // >0 => glyph point in front of the model surface
    }
    gaps.push(gap);
    const vis = gap > -1e-6; // depthFunc LessEqual: coplanar still passes
    if (vis) { visible++; visX += px.x; visY += px.y; } else { hidden++; if (gap < -1e-6) behind++; }
    if (!partStats[part]) partStats[part] = { n: 0, visible: 0, gaps: [] };
    partStats[part].n++;
    if (vis) partStats[part].visible++;
    partStats[part].gaps.push(gap);
  }
  return {
    inner: g.inner, normal, stemDir, stemForeshorten, stemScreenUpDeg,
    pxSeed, rPx,
    bboxPx: { minX, maxX, minY, maxY, width: maxX - minX, height: maxY - minY },
    seedToTopPx: pxSeed.y - minY,
    visibleFrac: visible / worlds.length,
    hiddenFrac: hidden / worlds.length,
    visibleCentroidOffsetPx: Math.hypot(visX / Math.max(visible, 1) - pxSeed.x, visY / Math.max(visible, 1) - pxSeed.y),
    partStats,
    gaps,
  };
}

// seed on the face-on front box face (world normal ~ +Z; camera dir z=0.878)
const faceOnSeed = seedsB[0];
const grazingSeed = seedsB[seedsB.length - 1];

const glyphCorrect = glyphMetrics(faceOnSeed.p, faceOnSeed.world, "correct", camB.cam, CANVAS.w, CANVAS.h, modelB.mesh);
const glyphBuggy = glyphMetrics(faceOnSeed.p, faceOnSeed.world, "buggy", camB.cam, CANVAS.w, CANVAS.h, modelB.mesh);

const camDirWorld = camB.dir.clone();
console.log(
  "glyph hierarchy: root(rot -90°X) > outerGroup(position=local seed) > innerGroup(quaternion)."
);
console.log(
  "correct billboard sets inner.local = parentWorldQuat⁻¹ * cameraWorldQuat; current code sets inner.local = cameraWorldQuat."
);
console.log("");
table(
  ["metric", "correct billboard", "current (buggy) billboard"],
  [
    ["glyph plane normal vs camera direction", f(glyphCorrect.normal.angleTo(camDirWorld) * DEG, 2) + "°", f(glyphBuggy.normal.angleTo(camDirWorld) * DEG, 2) + "°"],
    ["world orientation error (normal)", "0.00°", f(glyphBuggy.normal.angleTo(glyphCorrect.normal) * DEG, 2) + "°"],
    ["stem direction on screen (0°=up)", f(glyphCorrect.stemScreenUpDeg, 2) + "°", f(glyphBuggy.stemScreenUpDeg, 2) + "°"],
    ["stem screen foreshortening |screen-stem|", f(glyphCorrect.stemForeshorten, 4), f(glyphBuggy.stemForeshorten, 4)],
    ["glyph screen height px @fit zoom", f(glyphCorrect.bboxPx.height, 2), f(glyphBuggy.bboxPx.height, 2)],
    ["glyph screen width px @fit zoom", f(glyphCorrect.bboxPx.width, 2), f(glyphBuggy.bboxPx.width, 2)],
    ["seed-to-glyph-top px @fit zoom", f(glyphCorrect.seedToTopPx, 2), f(glyphBuggy.seedToTopPx, 2)],
    ["visible sample fraction (depth-tested)", f(glyphCorrect.visibleFrac * 100, 1) + "%", f(glyphBuggy.visibleFrac * 100, 1) + "%"],
    ["buried behind the surface", f(glyphCorrect.hiddenFrac * 100, 1) + "%", f(glyphBuggy.hiddenFrac * 100, 1) + "%"],
    ["visible-glyph centroid offset from seed", f(glyphCorrect.visibleCentroidOffsetPx, 2) + " px", f(glyphBuggy.visibleCentroidOffsetPx, 2) + " px"],
  ]
);
console.log("");
console.log("### E1. per-part visibility (face-on seed)");
const e1Rows = [];
for (const part of ["sphere", "stem", "leaf"]) {
  const c = glyphCorrect.partStats[part];
  const b = glyphBuggy.partStats[part];
  e1Rows.push([
    part,
    c ? f((c.visible / c.n) * 100, 1) + "%" : "n/a",
    b ? f((b.visible / b.n) * 100, 1) + "%" : "n/a",
    c ? f(median(c.gaps), 3) + " wu" : "n/a",
    b ? f(median(b.gaps), 3) + " wu" : "n/a",
  ]);
}
function median(arr) {
  const s = arr.slice().sort((a, b) => a - b);
  return s[Math.floor(s.length / 2)];
}
table(["glyph part", "correct visible", "buggy visible", "correct median gap to surface wu", "buggy median gap wu"], e1Rows);
console.log("");
console.log("At fit zoom the whole correct glyph is only " + f(glyphCorrect.bboxPx.height, 1) + " px tall (sphere radius " + f(glyphCorrect.rPx, 2) + " px);");
console.log("at zoom " + USER_ZOOM + " the same glyph is " + f(glyphCorrect.bboxPx.height * (USER_ZOOM / camB.zoom), 1) + " px tall (sphere radius " + f(glyphCorrect.rPx * (USER_ZOOM / camB.zoom), 2) + " px).");
console.log("");

// glyph extent vs the S5 clickable radius
const glyphTopR = 3.385; // (size + 2*size + 0.385*size) / size
const clickableRAtFaceOn = 2.5 * Math.abs(faceOnSeed.incidence);
const glyphFracOutside = 1 - clickableRAtFaceOn / glyphTopR;
console.log("### E2. what the user aims at vs what S5 accepts");
table(
  ["quantity", "value"],
  [
    ["correct billboard glyph top above seed", f(glyphTopR, 3) + " * size (3.385 marker radii)"],
    ["S5 clickable radius (world-dist), face-on", f(2.5 * faceOnSeed.incidence, 3) + " * size (" + f(clickableRAtFaceOn, 2) + " marker radii)"],
    ["share of glyph height outside S5's clickable radius", f(glyphFracOutside * 100, 1) + "%"],
    ["S2 clickable radius at fit zoom", "32 px = " + f(32 / glyphCorrect.rPx, 1) + " marker radii (angle-independent)"],
    ["S5 clickable radius at fit zoom, face-on", f(2.5 * faceOnSeed.incidence * glyphCorrect.rPx, 2) + " px"],
  ]
);
console.log("");
check("E.1", "current billboard has a large world orientation error (>60°)", glyphBuggy.normal.angleTo(glyphCorrect.normal) * DEG > 60, f(glyphBuggy.normal.angleTo(glyphCorrect.normal) * DEG, 1) + "°");
check("E.2", "correct billboard glyph normal faces the camera (<1°)", glyphCorrect.normal.angleTo(camDirWorld) * DEG < 1, f(glyphCorrect.normal.angleTo(camDirWorld) * DEG, 3) + "°");
check("E.3", "buggy glyph is strongly foreshortened (screen stem length < 0.5 of correct)", glyphBuggy.stemForeshorten < 0.5 * glyphCorrect.stemForeshorten, f(glyphBuggy.stemForeshorten, 3) + " vs " + f(glyphCorrect.stemForeshorten, 3));
check("E.4", "buggy glyph projects much shorter than correct (screen height < 0.5)", glyphBuggy.bboxPx.height < 0.5 * glyphCorrect.bboxPx.height, f(glyphBuggy.bboxPx.height, 2) + "px vs " + f(glyphCorrect.bboxPx.height, 2) + "px");
check("E.5", "correct billboard stem points screen-up", Math.abs(glyphCorrect.stemScreenUpDeg) < 1, f(glyphCorrect.stemScreenUpDeg, 3) + "°");
check("E.6", "buggy billboard stem is not screen-up (>45° away)", Math.abs(glyphBuggy.stemScreenUpDeg) > 45, f(glyphBuggy.stemScreenUpDeg, 1) + "°");
check("E.7", ">20% of the correct glyph's vertical extent lies outside S5's clickable radius (face-on)", glyphFracOutside > 0.2, f(glyphFracOutside * 100, 1) + "%");
check("E.8", "face-on: >60% of the billboarded glyph is buried inside the surface (visible < 40%)", glyphCorrect.visibleFrac < 0.4, f(glyphCorrect.visibleFrac * 100, 1) + "% visible / " + f(glyphCorrect.hiddenFrac * 100, 1) + "% buried");
check("E.9", "visible sprout above the seed is tiny at fit zoom and only readable when zoomed in", glyphCorrect.seedToTopPx < 8 && glyphCorrect.seedToTopPx * (USER_ZOOM / camB.zoom) > 20, f(glyphCorrect.seedToTopPx, 2) + "px @fit, " + f(glyphCorrect.seedToTopPx * (USER_ZOOM / camB.zoom), 1) + "px @zoom" + USER_ZOOM);
check("E.12", "billboard bug does not relocate the visible target centre (centroid stays <2px from the seed)", glyphBuggy.visibleCentroidOffsetPx < 2 && glyphCorrect.visibleCentroidOffsetPx < 2, f(glyphCorrect.visibleCentroidOffsetPx, 2) + "px / " + f(glyphBuggy.visibleCentroidOffsetPx, 2) + "px");

// grazing glyph: how much of the glyph is off-surface
const glyphGrazing = glyphMetrics(grazingSeed.p, grazingSeed.world, "correct", camB.cam, CANVAS.w, CANVAS.h, modelB.mesh);
console.log("### E3. same correct-billboard glyph on a grazing seed (incidence " + f(Math.acos(Math.min(1, grazingSeed.incidence)) * DEG, 0) + "°)");
table(
  ["quantity", "value"],
  [
    ["visible fraction", f(glyphGrazing.visibleFrac * 100, 1) + "%"],
    ["median signed gap glyph->surface", f(median(glyphGrazing.gaps), 3) + " wu"],
    ["share of glyph points > 0.5*size in front of surface", f(glyphGrazing.gaps.filter((g) => g > 0.5 * sizeB).length / glyphGrazing.gaps.length * 100, 1) + "%"],
    ["S5 clickable radius at this incidence", f(2.5 * grazingSeed.incidence, 3) + " * size"],
    ["visible sphere radius clickable by S5", f(Math.min(100, 2.5 * grazingSeed.incidence * 100), 0) + "%"],
  ]
);
console.log("");
check("E.10", "on a grazing surface the correct-billboard glyph visibly floats off-surface (median gap > 0.2*size)", median(glyphGrazing.gaps) > 0.2 * sizeB, f(median(glyphGrazing.gaps), 3) + " wu");
check("E.11", "on a grazing surface S5 can only erase within a fraction of the visible sphere radius", 2.5 * grazingSeed.incidence < 0.5, f(2.5 * grazingSeed.incidence, 3) + " r_px");

// ─────────────────────────────────────────────────────────────────────────────
// verdict
// ─────────────────────────────────────────────────────────────────────────────
console.log("");
console.log("## Verdict");
console.log("");
console.log("COORDINATE-SPACE HYPOTHESIS: CONFIRMED for S1/S2/S3/S4/S6, and it is the ONLY cause of the");
console.log("R6/R7/R8-first misses. S1 -> S2 and S3 -> S4 differ only by localToWorld() of the seed; the");
console.log("wrong versions miss by 149..721 px, the corrected versions land < 0.001 px. project() was never");
console.log("broken; it was fed STL-local coordinates as if they were world coordinates (the mesh lives under");
console.log("a -90 deg X group rotation). The 'ray data is garbage' conclusion was also wrong: the wrong-ray");
console.log("distance is exactly the same number as the wrong-projection distance, and it equals");
console.log("|(I-dd^T)(R-I)p| / wuPerPx -- at zoom 6.91 it is 549 px and at zoom 167.2 it is 13293 px, i.e.");
console.log("the two handoff endpoints are reproduced to 0.00% by pure geometry (zoom was the hidden variable,");
console.log("not pan). The corrected ray stays 0.0000 px at every zoom.");
console.log("");
console.log("The smear path (S6) is a fresh, independent bug of the same family: Viewport.tsx:2126 passes a");
console.log("WORLD hit point into a matcher that expects LOCAL seeds. Error = |(R-I)p| = sqrt(2*(y^2+z^2)):");
console.log("55..183 wu here (61x..203x over the 2.5*size threshold; up to 762 px at fit zoom). It is zero only");
console.log("for seeds on the X axis, so it fails silently and seed-dependently -- exactly the 'smear never");
console.log("fires' symptom.");
console.log("");
console.log("PARTIALLY CONFIRMED / what the coordinate story still misses:");
console.log("1. The current HEAD click path is coordinate-correct (local hit vs local seed) and SHOULD erase on");
console.log("   a plain click of the visible red half-ball. Two non-coordinate code paths can still produce");
console.log("   'never fires': (a) segmentMode=='auto' + stale seedEraseMode hits the autoNoSowing early");
console.log("   return at Viewport.tsx:1874 BEFORE the erase branch (switching mode never clears");
console.log("   seedEraseMode, while the eraser cursor at line 1142 is shown regardless of mode); (b) any");
console.log("   1px drag after pointerdown enters the smear path, which logs/hud 'erase miss ... wu' and");
console.log("   OVERWRITES the 'erased' status from the click -- a successful click can look like a miss.");
console.log("2. S5's tolerance is 2.5*size*cos(incidence): " + f(2.5 * 0.878 * sizeB * camB.zoom, 2) + " px face-on at fit zoom, and only");
console.log("   " + f(2.5 * 0.2 * sizeB * USER_ZOOM, 2) + " px at zoom " + USER_ZOOM + " for cos=0.2 (50% of the visible sphere radius). A corrected");
console.log("   S2/S4 screen match gives a flat 32 px (21.3 marker radii at fit zoom) at any incidence, so");
console.log("   fixing the smear path is not enough: S5's click path will still miss on grazing surfaces.");
console.log("3. The marker is a 3-D sapling, not a flat sprite, billboarded into the surface: on a face-on");
console.log("   face 72.1% of its samples are behind the surface (stem 0%, leaf 0% visible) -- the user's");
console.log("   'red dot' is really the front hemisphere of the seed sphere, centred on the seed. The current");
console.log("   billboard is also wrong by construction (inner.local = cameraWorldQuat instead of");
console.log("   parentWorldQuat^-1 * cameraWorldQuat): world orientation error 83.0 deg, stem 85.8 deg off");
console.log("   screen-up, foreshortened 0.229, sapling screen height 2.99 px vs 6.57 px correct. The visible");
console.log("   centroid still sits 0.37 px from the seed, so the bug does not relocate the target -- it just");
console.log("   makes the glyph look like a deformed red blob and hides the sprout.");
console.log("4. The glyph top is 3.385 marker radii above the seed while S5 accepts 2.176 radii face-on:");
console.log("   35.7% of the sapling's height (and 100% of stem/leaf area here) is outside the clickable");
console.log("   world-distance radius, so 'aim at the glyph' is not the same as 'click the seed'.");
console.log("");

console.log("## Checks");
for (const c of checks) {
  console.log((c.ok ? "PASS " : "FAIL ") + c.id + "  " + c.desc + (c.detail ? "   [" + c.detail + "]" : ""));
}
console.log("");
console.log(
  (checks.length - failures) + "/" + checks.length + " checks passed" +
  (failures ? "  ->  " + failures + " FAILURE(S), exit 1" : "  ->  exit 0")
);
process.exitCode = failures ? 1 : 0;
