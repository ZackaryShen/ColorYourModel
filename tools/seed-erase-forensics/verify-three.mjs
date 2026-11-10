#!/usr/bin/env node
/**
 * verify-three.mjs — headless proof that the page's REAL three.js path agrees
 * with core.mjs, plus a numeric check of the two "erase matching" modes.
 *
 * No WebGL / no DOM needed: three.js only supplies the camera matrices, the
 * Raycaster and the world matrix — exactly the parts the page uses.
 *
 *   node verify-three.mjs [--model=box|dome|knot] [--seeds=5]
 *
 * Exit 0 = every check passed.
 */

import * as THREE from "./node_modules/three/build/three.module.js";
import * as C from "./core.mjs";

function arg(flag, fallback) {
  const hit = process.argv.slice(2).find((a) => a.startsWith(`--${flag}=`));
  return hit ? hit.split("=")[1] : fallback;
}

const modelId = arg("model", "dome");
const seedCount = Math.max(1, Number(arg("seeds", "5")) || 5);
const viewport = { ...C.CANONICAL_VIEWPORT };
const model = C.buildModel(modelId);
const cam = C.fitCamera(model.bbox, viewport);
const { size } = C.seedMarkerSize(model.bbox);
const threshold = C.eraseThresholdWorld(size);
const tolPx = threshold * cam.zoom;

// ── exactly what app.mjs builds ─────────────────────────────────────────────
const cam3 = new THREE.OrthographicCamera(
  cam.frustum.left, cam.frustum.right, cam.frustum.top, cam.frustum.bottom, cam.near, cam.far
);
cam3.zoom = cam.zoom;
cam3.up.set(0, 1, 0);
cam3.position.set(...cam.position);
cam3.lookAt(...cam.target);
cam3.updateProjectionMatrix();
cam3.updateMatrixWorld(true);

const pos = new Float32Array(model.faces.length * 3);
for (let f = 0; f < model.faces.length; f++) {
  const vi = model.faces[f] * 3;
  pos[f * 3] = model.vertices[vi];
  pos[f * 3 + 1] = model.vertices[vi + 1];
  pos[f * 3 + 2] = model.vertices[vi + 2];
}
const geo = new THREE.BufferGeometry();
geo.setAttribute("position", new THREE.Float32BufferAttribute(pos, 3));
geo.computeVertexNormals();
const material = new THREE.MeshLambertMaterial({ color: 0x9aa4b0, side: THREE.DoubleSide });
const mesh = new THREE.Mesh(geo, material);
const group = new THREE.Group(); // Viewport.tsx:2580
group.rotation.set(C.GROUP_ROT_X, 0, 0);
group.add(mesh);
group.updateMatrixWorld(true);

const raycaster = new THREE.Raycaster();
let lastRay = null;

/** getLocalHit(): Viewport.tsx:1425-1462, verbatim semantics. */
function threeHitAt(px, py) {
  mesh.updateMatrixWorld(true);
  const ndcX = (px / viewport.width) * 2 - 1;
  const ndcY = -((py / viewport.height) * 2) + 1;
  raycaster.setFromCamera(new THREE.Vector2(ndcX, ndcY), cam3);
  lastRay = {
    origin: raycaster.ray.origin.clone(),
    direction: raycaster.ray.direction.clone(),
  };
  const hits = raycaster.intersectObject(mesh, false);
  if (!hits.length) return null;
  const h = hits[0];
  const local = mesh.worldToLocal(h.point.clone());
  return {
    faceIndex: h.faceIndex ?? -1,
    world: [h.point.x, h.point.y, h.point.z],
    local: [local.x, local.y, local.z],
    distance: h.distance,
  };
}

const check = (name, ok, detail) => {
  results.push({ name, ok, detail });
  console.log(` ${ok ? "PASS" : "FAIL"}  ${name}${detail ? ` — ${detail}` : ""}`);
};
const results = [];
const maxD = (a, b) => Math.max(a, b);

console.log("═".repeat(110));
console.log(` verify-three.mjs — three.js r${THREE.REVISION} path vs core.mjs   model=${modelId} faces=${model.faceCount}`);
console.log(` viewport=${viewport.width}x${viewport.height} zoom=${cam.zoom.toFixed(3)} worldPerPx=${cam.worldPerPx.toFixed(5)}`);
console.log(` S5 tolerance = ${threshold.toFixed(4)} wu ≈ ${tolPx.toFixed(2)} px   S2/S4 = 32 px`);
console.log("═".repeat(110));

// ── 1) camera basis ────────────────────────────────────────────────────────
{
  const m = cam3.matrixWorld.elements;
  const d = maxD(
    maxD(C.dist3(cam.xAxis, [m[0], m[1], m[2]]), C.dist3(cam.yAxis, [m[4], m[5], m[6]])),
    C.dist3(cam.zAxis, [m[8], m[9], m[10]])
  );
  check("camera basis == THREE.Matrix4.lookAt", d < 1e-12, `max Δ ${d.toExponential(1)}`);
}

// ── 2) seed placement (three raycast), projection + ray + hit agreement ────
const placed = C.autoPlaceSeeds({ mesh: model, cam, count: seedCount, markerSize: size, raycastAt: threeHitAt });
check("auto-place found clickable seeds", placed.length === seedCount, `${placed.length}/${seedCount} seeds (three.js raycast)`);

let dProj = 0;
const v = new THREE.Vector3();
for (const s of placed) {
  v.set(...s.world).project(cam3);
  const threePx = [(v.x * 0.5 + 0.5) * viewport.width, (1 - (v.y * 0.5 + 0.5)) * viewport.height];
  const corePx = C.projectWorld(s.world, cam);
  dProj = maxD(dProj, Math.hypot(threePx[0] - corePx[0], threePx[1] - corePx[1]));
}
check("Vector3.project(camera) == core.projectWorld", dProj < 1e-9, `max Δ ${dProj.toExponential(1)} px`);

{
  const px = Math.round(C.projectWorld(placed[0].world, cam)[0]);
  const py = Math.round(C.projectWorld(placed[0].world, cam)[1]);
  const th = threeHitAt(px, py);
  const coreRay = C.rayFromScreen(px, py, cam);
  const dO = C.dist3(coreRay.origin, [lastRay.origin.x, lastRay.origin.y, lastRay.origin.z]);
  const dD = C.dist3(coreRay.direction, [lastRay.direction.x, lastRay.direction.y, lastRay.direction.z]);
  check("Raycaster.setFromCamera(ortho) == core.rayFromScreen", maxD(dO, dD) < 1e-12, `origin Δ ${dO.toExponential(1)} dir Δ ${dD.toExponential(1)}`);
  const ch = C.raycastMesh(coreRay, model);
  const dHit = th && ch ? maxD(C.dist3(th.world, ch.world), C.dist3(th.local, ch.local)) : th === ch ? 0 : Infinity;
  // Not bit-exact by design: three intersects the Float32 position buffer the app
  // uploads (useMesh.buildBaseGeometry: new Float32Array(meshData.vertices)),
  // while core.mjs uses the original float64 arrays.
  check(
    "Raycaster hit == core Möller–Trumbore",
    dHit < 1e-5,
    `Δ ${dHit.toExponential(1)} wu (face ${th && ch ? th.faceIndex : "—"}) — float32 position buffer vs float64 source`
  );
}

// ── 3) billboard bug measured through three's world quaternions ───────────
{
  const camQ = cam3.quaternion.clone();
  const outer = new THREE.Group();
  outer.position.set(...placed[0].local);
  const inner = new THREE.Group();
  outer.add(inner);
  group.add(outer);
  // bug: Viewport.tsx:2789 — copy the camera's WORLD quaternion into the child
  inner.quaternion.copy(camQ);
  group.updateMatrixWorld(true);
  const worldBug = new THREE.Quaternion();
  inner.getWorldQuaternion(worldBug);
  const ang = (q) => (2 * Math.acos(Math.min(1, Math.abs(q.dot(camQ)))) * 180) / Math.PI;
  const tiltBug = ang(worldBug);
  // corrected: parent⁻¹ · camera
  const parentInv = new THREE.Quaternion();
  group.getWorldQuaternion(parentInv).invert();
  inner.quaternion.copy(camQ).premultiply(parentInv);
  group.updateMatrixWorld(true);
  inner.getWorldQuaternion(worldBug);
  const tiltFix = ang(worldBug);
  check("billboard bug reproduces 90° off camera", Math.abs(tiltBug - 90) < 1e-6, `${tiltBug.toFixed(6)}° (bug)`);
  check("corrected billboard is 0° off camera", tiltFix < 1e-6, `${tiltFix.toFixed(6)}° (parent⁻¹·camera)`);
  group.remove(outer);
}

// ── 4) the two-scenario self-test through the real three.js path ──────────
const scenarios = C.evaluateScenarios({ mesh: model, cam, seeds: placed, markerSize: size, raycastAt: threeHitAt });
const report = C.buildReport({ mesh: model, cam, viewport, seeds: placed, scenarios });
const coreReport = C.runSelfTest({ modelId, viewport, seedCount });
console.log("");
console.log(report.text);
console.log("");
check(
  "two-scenario self-test (three.js path) matches expectations",
  report.expectedVsActual.allOk,
  report.scenarios.map((s) => `${s.id}:${s.allOk ? "PASS" : "FAIL"}`).join(" ")
);
check(
  "three.js path verdicts == core-only probe verdicts",
  JSON.stringify(report.scenarios.map((s) => s.aggregate.map((a) => [a.id, a.actual]))) ===
    JSON.stringify(coreReport.scenarios.map((s) => s.aggregate.map((a) => [a.id, a.actual]))),
  "identical per-strategy verdicts in both scenarios"
);

// ── 5) the "erase matching" claim ─────────────────────────────────────────
// For every placed seed, click offset px to the right of ITS rendered marker and
// ask each matcher which seed it would remove. A matcher is correct for that
// click iff it names the seed that was clicked.
console.log("");
console.log(" ERASE MATCHING — click offsets from the rendered marker (three.js raycast)");
console.log(` ${"offset".padEnd(8)} ${"S5 (HEAD) — seed found".padEnd(30)} ${"S2 (corrected) — seed found".padEnd(32)}`);
let s2Reliable = true;
let s5Brittle = false;
for (const offset of [0, 1, 2, 3, 4, 8, 16, 24, 31, 35, 40]) {
  let s5ok = 0;
  let s2ok = 0;
  for (let i = 0; i < placed.length; i++) {
    const seed = placed[i];
    const proj = C.projectWorld(seed.world, cam);
    const px = [Math.round(proj[0]) + offset, Math.round(proj[1])];
    const hit = threeHitAt(px[0], px[1]);
    const ev = C.evaluateFromHit({ clickPx: px, cam, seedsLocal: placed.map((s) => s.local), markerSize: size, hit });
    const s5 = ev.rows.find((r) => r.id === "S5");
    const s2 = ev.rows.find((r) => r.id === "S2");
    if (s5.pass && s5.nearestSeed === i) s5ok++;
    if (s2.pass && s2.nearestSeed === i) s2ok++;
  }
  if (offset <= 31 && s2ok !== placed.length) s2Reliable = false;
  if (offset >= 4 && offset <= 31 && s5ok < placed.length) s5Brittle = true;
  console.log(
    ` ${String(`+${offset}px`).padEnd(8)} ` +
      `${s5ok === placed.length ? `all ${placed.length} markers` : `${s5ok}/${placed.length} (misses ${placed.length - s5ok})`.padEnd(28)} ` +
      `${s2ok === placed.length ? `all ${placed.length} markers` : `${s2ok}/${placed.length}`.padEnd(30)}`
  );
}
check(
  "S2 matcher erases reliably for any aim within 32 px",
  s2Reliable,
  "every marker recognised at 0..31 px offsets"
);
check(
  "S5 matcher at HEAD is brittle off-centre",
  s5Brittle,
  "4..31 px offsets already miss — the reported 'never fires' feel"
);

// Off-surface click (the sapling sticks out of the mesh; markers are not
// raycastable, so getLocalHit() returns null while the pointer is visually on
// the glyph): S2 must still erase, S5 cannot run at all.
{
  const seed = placed[0];
  const proj = C.projectWorld(seed.world, cam);
  const px = [Math.round(proj[0]), Math.round(proj[1])];
  const ev = C.evaluateFromHit({
    clickPx: px,
    cam,
    seedsLocal: placed.map((s) => s.local),
    markerSize: size,
    hit: null, // simulate the pointer being on the glyph, off the surface
  });
  const s2 = ev.rows.find((r) => r.id === "S2");
  const s5 = ev.rows.find((r) => r.id === "S5");
  // Does such a pixel actually exist near this marker in this view?
  let noHitPx = null;
  for (let dy = 1; dy < 30 && !noHitPx; dy++) {
    for (const dx of [0, 2, -2, 4, -4]) {
      const q = [Math.round(proj[0]) + dx, Math.round(proj[1]) - dy];
      if (!threeHitAt(q[0], q[1])) {
        noHitPx = q;
        break;
      }
    }
  }
  check(
    "off-surface click: S2 still erases, S5 cannot run",
    s2.pass && s2.nearestSeed === 0 && s5.unavailable != null,
    noHitPx
      ? `real no-hit pixel found ${Math.hypot(noHitPx[0] - proj[0], noHitPx[1] - proj[1]).toFixed(1)} px from the marker; S2=${s2.pass ? "ERASES" : "misses"}, S5=${s5.unavailable ? "N/A" : "runs"}`
      : `simulated hit=null at the marker: S2=${s2.pass ? "ERASES" : "misses"}, S5=${s5.unavailable ? "N/A" : "runs"} (no real no-hit pixel within 30 px in this view)`
  );
}

// ── summary ───────────────────────────────────────────────────────────────
const failed = results.filter((r) => !r.ok);
console.log("");
console.log("═".repeat(110));
console.log(
  failed.length === 0
    ? ` ALL ${results.length} CHECKS PASSED — three.js path ≡ core.mjs, both scenarios as expected, S2 matcher reliable, S5 brittle`
    : ` ${failed.length}/${results.length} CHECKS FAILED: ${failed.map((f) => f.name).join(" | ")}`
);
console.log("═".repeat(110));
process.exit(failed.length === 0 ? 0 : 1);
