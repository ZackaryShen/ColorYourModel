/**
 * app.mjs — the browser exhibit.
 *
 * Renders the real geometry (rotated scene group, Z-up mesh, orthographic
 * CameraFit, the sapling glyph, the tiny erase radius) and evaluates all six
 * strategies per click. Every number shown comes from core.mjs, which the page
 * cross-checks against the real three.js engine on every click — so what you
 * read in the table is what the app's own Raycaster/camera actually produce.
 */

import * as THREE from "./node_modules/three/build/three.module.js";
import * as C from "./core.mjs";

// ─── DOM helpers ────────────────────────────────────────────────────────────
const $ = (id) => document.getElementById(id);
const esc = (s) =>
  String(s ?? "").replace(/[&<>"]/g, (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[ch]));
const fmt = (n, d = 2) => (n == null || !Number.isFinite(n) ? "—" : (Math.abs(n) < 1e-9 ? 0 : n).toFixed(d));
const triple = (t, d = 2) => (t ? `(${t.map((v) => fmt(v, d)).join(", ")})` : "—");
const vcls = (v) => (v === "PASS" ? "v--pass" : v === "FAIL" ? "v--fail" : "v--warn");
// Timer-based yield: resolves in real browsers AND in headless --dump-dom runs,
// where requestAnimationFrame is not guaranteed to fire without a compositor.
const nextFrame = () => new Promise((r) => setTimeout(r, 20));

const el = {
  wrap: $("stageWrap"),
  gl: $("gl"),
  overlay: $("overlay"),
  facts: $("facts"),
  modelSel: $("modelSel"),
  seedCount: $("seedCount"),
  btnPlace: $("btnPlace"),
  btnCenter: $("btnCenter"),
  btnClear: $("btnClear"),
  btnFit: $("btnFit"),
  billboardSel: $("billboardSel"),
  chkMutate: $("chkMutate"),
  chkOverlay: $("chkOverlay"),
  chkInset: $("chkInset"),
  modeGroup: $("modeGroup"),
  matcherGroup: $("matcherGroup"),
  matcherChip: $("matcherChip"),
  matcherChoices: $("matcherChoices"),
  matcherHint: $("matcherHint"),
  s5Desc: $("s5Desc"),
  s2Desc: $("s2Desc"),
  stageNote: $("stageNote"),
  tblClick: $("tblClick"),
  clickMeta: $("clickMeta"),
  clickNote: $("clickNote"),
  smearLine: $("smearLine"),
  clickLog: $("clickLog"),
  xcheck: $("xcheck"),
  btnSelfTest: $("btnSelfTest"),
  btnCopy: $("btnCopy"),
  stStatus: $("stStatus"),
  stTables: $("stTables"),
  stDetail: $("stDetail"),
  stDetails: $("stDetails"),
  toast: $("toast"),
  dropHint: $("dropHint"),
};

// ─── SeedMarker glyph, transcribed (Viewport.tsx:2704-2760) ─────────────────
const SEED_GEOMETRY = {
  sphere: new THREE.SphereGeometry(1, 10, 10),
  stem: new THREE.CylinderGeometry(0.16, 0.16, 2, 6),
  leaf: new THREE.SphereGeometry(0.385, 8, 8),
};
const seedMaterialCache = new Map();
function seedMaterial(ghost, erase) {
  const key = `${ghost}-${erase}`;
  let m = seedMaterialCache.get(key);
  if (!m) {
    const color = erase ? "#ff4d4f" : ghost ? "#ff2bd6" : "#3ddc84";
    m = new THREE.MeshBasicMaterial({
      color,
      depthTest: !ghost,
      transparent: ghost,
      opacity: ghost ? 0.85 : 1,
    });
    seedMaterialCache.set(key, m);
  }
  return m;
}

// ─── state ──────────────────────────────────────────────────────────────────
const state = {
  modelId: "dome",
  model: null, // core mesh {vertices, faces, bbox, faceCount, name, label, note}
  gl: null,
  scene: null,
  group: null, // rotated -PI/2 about X — Viewport.tsx:2580
  mesh3: null,
  grid: null,
  wire: null,
  markers: [], // {outer, inner}
  parentInv: new THREE.Quaternion(),
  seeds: [], // MODEL-LOCAL triples
  viewport: { width: 900, height: 620 },
  fit: null, // fitCamera result (fit facts)
  cam: null, // live core camera
  cam3: null, // live three camera (the app's real camera)
  insetCam: null,
  insetCam3: null,
  insetRect: null,
  mode: "erase",
  matcher: "s5", // "s5" = app-faithful at HEAD; "s2" = corrected
  billboard: "bug",
  mutate: true,
  overlay: true,
  inset: true,
  lastEval: null, // per-click snapshot (safe if the seed is erased)
  lastLog: [],
  smear: { attempts: 0, hits: 0, last: null },
  erasing: false,
  orbiting: null,
  selfTest: null,
  xcheck: null,
  raycaster: new THREE.Raycaster(),
  lastRay: null,
  sizes: { size: 0, thresh: 0, diag: 0 },
  dpr: 1,
  toastTimer: 0,
};

const INSET_MAG = 8;

// ─── init ──────────────────────────────────────────────────────────────────
init();

function init() {
  state.gl = new THREE.WebGLRenderer({ canvas: el.gl, antialias: true });
  state.gl.setClearColor(0x2a2a2a, 1); // var(--bg-canvas, #2a2a2a)
  state.gl.setPixelRatio(Math.min(2, window.devicePixelRatio || 1));

  state.scene = new THREE.Scene();
  // SceneSetup (Viewport.tsx:2863-2871): ambient 0.4 + two directional lights.
  state.scene.add(new THREE.AmbientLight(0xffffff, 0.4));
  const d1 = new THREE.DirectionalLight(0xffffff, 0.8);
  d1.position.set(10, 10, 5);
  state.scene.add(d1);
  const d2 = new THREE.DirectionalLight(0xffffff, 0.3);
  d2.position.set(-5, -5, -5);
  state.scene.add(d2);

  // Model presets
  for (const m of C.MODELS) {
    const opt = document.createElement("option");
    opt.value = m.id;
    opt.textContent = m.label;
    el.modelSel.appendChild(opt);
  }
  el.modelSel.value = state.modelId;

  // Seed markers live INSIDE the rotated group, next to the mesh.
  state.group = new THREE.Group();
  state.group.rotation.set(C.GROUP_ROT_X, 0, 0);
  state.scene.add(state.group);

  bindEvents();
  window.addEventListener("resize", onResize);
  // The canvas must track the element even when only the panel/scroller change
  // the available width (e.g. a scrollbar appearing).
  if (typeof ResizeObserver !== "undefined") {
    new ResizeObserver(() => onResize()).observe(el.wrap);
  }
  layoutViewport();
  requestAnimationFrame(tick);
  // Two frames: let the app shell (header wrap, panel scrollbars) settle before
  // CameraFit runs, so the canvas buffer and the fitted camera use the same size.
  requestAnimationFrame(() =>
    requestAnimationFrame(() => {
      layoutViewport();
      loadModel(C.buildModel(state.modelId), { autoSeeds: true, fit: true });
      applyMatcher();
      // First impression: run the self-test immediately so the table is populated.
      setTimeout(() => runSelfTest(true), 260);
    })
  );

  // Debug/verification API (used by the headless checks and by curious readers).
  window.__lab = {
    state,
    core: C,
    three: THREE,
    threeHitAt,
    autoPlace,
    resetView,
    runSelfTest,
    clickAt: (px, py) => handleLeftClick(px, py),
    eraseMatch,
  };
  if (new URLSearchParams(location.search).has("smoke")) setTimeout(runSmoke, 900);
}

/**
 * ?smoke=1 — deterministic end-to-end check of the two "erase matching" modes
 * through the page's real event path. Result lands in #smokeOut and
 * window.__smoke; the document title becomes "SMOKE PASS" / "SMOKE FAIL".
 */
function runSmoke() {
  const out = { checks: [] };
  const add = (name, ok, detail) => out.checks.push({ name, ok: !!ok, detail: detail ?? "" });
  const markerPx = (i) => {
    const p = C.projectWorld(C.localToWorld(state.seeds[i]), state.cam);
    return [Math.round(p[0]), Math.round(p[1])];
  };
  const setup = (n) => {
    resetView();
    state.seeds = [];
    state.lastEval = null;
    rebuildMarkers();
    autoPlace(n);
  };
  try {
    // 1. S2 matcher: 6 px right of the marker must still erase exactly that seed.
    setup(3);
    const want = state.seeds[0].slice();
    let px = markerPx(0);
    px = [px[0] + 6, px[1]];
    state.matcher = "s2";
    applyMatcher();
    handleLeftClick(px[0], px[1]);
    const gone = !state.seeds.some((s) => C.dist3(s, want) < 1e-9);
    add("S2 matcher: +6 px click erases the intended seed", state.seeds.length === 2 && gone, `seeds=${state.seeds.length} (expected 2), removed=${gone}`);

    // 2. S5 matcher (HEAD): the same +6 px click must erase nothing.
    setup(3);
    px = markerPx(0);
    px = [px[0] + 6, px[1]];
    state.matcher = "s5";
    applyMatcher();
    handleLeftClick(px[0], px[1]);
    add("S5 matcher (HEAD): +6 px click erases nothing", state.seeds.length === 3, `seeds=${state.seeds.length} (expected 3)`);

    // 3. S5 matcher: an exact click on the marker still works when it lands on the surface.
    setup(3);
    px = markerPx(0);
    state.matcher = "s5";
    applyMatcher();
    const before = state.seeds.length;
    handleLeftClick(px[0], px[1]);
    add("S5 matcher (HEAD): exact click erases", state.seeds.length === before - 1, `seeds=${state.seeds.length} (expected ${before - 1})`);

    // 4. S2 matcher: an off-surface click (no mesh hit) still erases.
    setup(3);
    px = markerPx(0);
    // find a no-hit pixel within 32 px of the marker (the glyph sticks out)
    let noHit = null;
    for (let dy = 1; dy < 32 && !noHit; dy++) {
      for (const dx of [0, 2, -2, 4, -4]) {
        const q = [px[0] + dx, px[1] - dy];
        if (!threeHitAt(q[0], q[1])) {
          noHit = q;
          break;
        }
      }
    }
    if (noHit) {
      state.matcher = "s2";
      applyMatcher();
      handleLeftClick(noHit[0], noHit[1]);
      add(
        "S2 matcher: off-surface click (getLocalHit=null) still erases",
        state.seeds.length === 2,
        `clicked px=(${noHit[0]}, ${noHit[1]}) ${Math.hypot(noHit[0] - px[0], noHit[1] - px[1]).toFixed(1)} px from marker, no mesh hit, seeds=${state.seeds.length}`
      );
    } else {
      add("S2 matcher: off-surface click (getLocalHit=null) still erases", true, "no no-hit pixel within 32 px in this view — mechanism covered by verify-three.mjs");
    }
  } catch (err) {
    out.error = String(err && err.stack ? err.stack : err);
  }
  out.ok = out.checks.length > 0 && out.checks.every((c) => c.ok);
  window.__smoke = out;
  document.title = `SMOKE ${out.ok ? "PASS" : "FAIL"}`;
  // Restore the exhibit to its default state.
  try {
    state.matcher = "s5";
    applyMatcher();
    runSelfTest(false);
  } catch { /* ignore */ }
  const box = document.createElement("pre");
  box.id = "smokeOut";
  box.style.display = "none";
  box.textContent = JSON.stringify(out, null, 2);
  document.body.appendChild(box);
}

// ─── model / scene ─────────────────────────────────────────────────────────
function buildRenderGeometry(model) {
  const { vertices, faces } = model;
  const count = faces.length;
  const pos = new Float32Array(count * 3);
  for (let f = 0; f < count; f++) {
    const vi = faces[f] * 3;
    pos[f * 3] = vertices[vi];
    pos[f * 3 + 1] = vertices[vi + 1];
    pos[f * 3 + 2] = vertices[vi + 2];
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute("position", new THREE.Float32BufferAttribute(pos, 3));
  geo.computeVertexNormals(); // non-indexed ⇒ per-face normals (flat look, like the app)
  geo.computeBoundingSphere();
  return geo;
}

function disposeModel() {
  if (state.mesh3) {
    state.group.remove(state.mesh3);
    state.mesh3.geometry.dispose();
    state.mesh3.material.dispose();
    state.mesh3 = null;
  }
  if (state.wire) {
    state.group.remove(state.wire);
    state.wire.geometry.dispose();
    state.wire.material.dispose();
    state.wire = null;
  }
  if (state.grid) {
    state.scene.remove(state.grid);
    state.grid.geometry.dispose();
    state.grid.material.dispose();
    state.grid = null;
  }
  for (const m of state.markers) state.group.remove(m.outer);
  state.markers = [];
}

function loadModel(model, { autoSeeds = false, fit = true } = {}) {
  disposeModel();
  state.model = model;
  state.seeds = [];
  state.lastEval = null;
  state.selfTest = null;
  state.xcheck = null;
  state.lastLog = [];
  state.smear = { attempts: 0, hits: 0, last: null };

  const geo = buildRenderGeometry(model);
  const mat = new THREE.MeshLambertMaterial({ color: 0x9aa4b0, side: THREE.DoubleSide });
  state.mesh3 = new THREE.Mesh(geo, mat);
  state.mesh3.castShadow = true;
  state.mesh3.receiveShadow = true;
  state.group.add(state.mesh3);

  // Wireframe overlay, as in the app for <=50k faces (Viewport.tsx:1119).
  if (model.faceCount <= 20000) {
    const wgeo = new THREE.WireframeGeometry(geo);
    const wmat = new THREE.LineBasicMaterial({ color: 0x333333, opacity: 0.05, transparent: true });
    state.wire = new THREE.LineSegments(wgeo, wmat);
    state.group.add(state.wire);
  }

  // AdaptiveGrid (Viewport.tsx:346-367) — world-space grid on the model base.
  const { min, max } = model.bbox;
  const spanX = max[0] - min[0];
  const spanY = max[1] - min[1];
  const gridSize = Math.ceil((Math.max(spanX, spanY) * 2.5) / 10) * 10 || 200;
  const centerX = (min[0] + max[0]) / 2;
  const centerZ = -(min[1] + max[1]) / 2;
  const divisions = Math.max(10, Math.min(40, Math.ceil(gridSize / 10)));
  state.grid = new THREE.GridHelper(gridSize, divisions, 0x555555, 0x333333);
  state.grid.position.set(centerX, 0, centerZ);
  state.scene.add(state.grid);

  state.sizes = (() => {
    const s = C.seedMarkerSize(model.bbox);
    return { size: s.size, thresh: C.eraseThresholdWorld(s.size), diag: s.diag };
  })();

  if (fit) {
    resetView();
  }
  renderFacts();
  renderSmear();
  renderClickTable(null);
  el.clickMeta.textContent = "— no click yet —";
  el.clickNote.textContent =
    state.mode === "erase"
      ? "Erase mode: click a red sapling. The panel shows which of the six strategies would remove the seed at that pixel."
      : "Sow mode: click the model — the seed is stored at the clicked face's CENTROID, in model-local coordinates.";
  el.stTables.innerHTML =
    '<div class="scen-empty muted">press “Auto self-test” — both aim scenarios will be filled in side by side</div>';
  el.stStatus.className = "status";
  el.stStatus.textContent = "not run yet";
  if (autoSeeds) autoPlace(seedCountValue());

  if (state.mode === "erase") applyMarkerColors();
  applyMatcher();
}

function resetView() {
  state.cam = C.fitCamera(state.model.bbox, state.viewport);
  state.fit = state.cam;
  applyCamera();
}

function applyCamera() {
  const cam = state.cam;
  const c = state.cam3;
  if (!c) {
    state.cam3 = new THREE.OrthographicCamera(
      cam.frustum.left,
      cam.frustum.right,
      cam.frustum.top,
      cam.frustum.bottom,
      cam.near,
      cam.far
    );
    state.cam3 = state.cam3;
  }
  const cam3 = state.cam3;
  cam3.left = cam.frustum.left;
  cam3.right = cam.frustum.right;
  cam3.top = cam.frustum.top;
  cam3.bottom = cam.frustum.bottom;
  cam3.near = cam.near;
  cam3.far = cam.far;
  cam3.zoom = cam.zoom;
  cam3.up.set(0, 1, 0);
  cam3.position.set(cam.position[0], cam.position[1], cam.position[2]);
  cam3.lookAt(cam.target[0], cam.target[1], cam.target[2]);
  cam3.updateProjectionMatrix();
  cam3.updateMatrixWorld(true);
  cam3.updateMatrixWorldInverse?.();
  renderFacts();
}

// ─── resize / viewport ─────────────────────────────────────────────────────
function onResize() {
  if (!layoutViewport()) return;
  // CameraFit deliberately does NOT depend on viewport size (Viewport.tsx:252),
  // so a resize only re-derives the frustum — the framing stays put.
  if (state.cam) {
    state.cam = {
      ...state.cam,
      width: state.viewport.width,
      height: state.viewport.height,
      frustum: {
        left: -state.viewport.width / 2,
        right: state.viewport.width / 2,
        top: state.viewport.height / 2,
        bottom: -state.viewport.height / 2,
      },
    };
    applyCamera();
    renderStageNote();
  }
}

/** Re-derive the drawing-buffer size from the element's real CSS size. */
function layoutViewport() {
  const rect = el.wrap.getBoundingClientRect();
  const w = Math.max(320, Math.round(rect.width));
  const h = Math.max(240, Math.round(rect.height));
  if (w === state.viewport.width && h === state.viewport.height) return false;
  state.viewport = { width: w, height: h };
  state.dpr = Math.min(2, window.devicePixelRatio || 1);
  state.gl.setPixelRatio(state.dpr);
  state.gl.setSize(w, h, false);
  el.overlay.width = Math.round(w * state.dpr);
  el.overlay.height = Math.round(h * state.dpr);
  return true;
}

// ─── markers (exact SeedMarker structure) ──────────────────────────────────
function applyMarkerColors() {
  const mat = seedMaterial(false, state.mode === "erase");
  for (const m of state.markers) {
    m.inner.children.forEach((child) => (child.material = mat));
  }
}

function rebuildMarkers() {
  for (const m of state.markers) state.group.remove(m.outer);
  state.markers = [];
  const size = state.sizes.size;
  const r = size;
  const stemH = size * 2.0;
  const leafR = size * 0.55;
  const mat = seedMaterial(false, state.mode === "erase");

  for (const seed of state.seeds) {
    // <group position={pos}> <group ref> … </group> </group>
    const outer = new THREE.Group();
    outer.position.set(seed[0], seed[1], seed[2]);
    const inner = new THREE.Group();
    const sphere = new THREE.Mesh(SEED_GEOMETRY.sphere, mat);
    sphere.scale.setScalar(r);
    const stem = new THREE.Mesh(SEED_GEOMETRY.stem, mat);
    stem.scale.setScalar(size);
    stem.position.set(0, r + stemH / 2, 0);
    const leaf1 = new THREE.Mesh(SEED_GEOMETRY.leaf, mat);
    leaf1.scale.setScalar(size);
    leaf1.position.set(leafR, r + stemH, 0);
    const leaf2 = new THREE.Mesh(SEED_GEOMETRY.leaf, mat);
    leaf2.scale.setScalar(size);
    leaf2.position.set(-leafR, r + stemH, 0);
    inner.add(sphere, stem, leaf1, leaf2);
    outer.add(inner);
    state.group.add(outer);
    state.markers.push({ outer, inner });
  }
  // The group's world quaternion (constant) is needed for the corrected billboard.
  state.group.updateMatrixWorld(true);
  state.group.getWorldQuaternion(state.parentInv).invert();
  updateBillboard();
  renderStageNote();
}

function updateBillboard() {
  if (!state.cam3) return;
  const camQ = state.cam3.quaternion;
  for (const m of state.markers) {
    if (state.billboard === "bug") {
      // Viewport.tsx:2789 — g.quaternion.copy(cam.quaternion) on a CHILD of the
      // rotated group: the resulting WORLD orientation is R_group · Q_camera.
      m.inner.quaternion.copy(camQ);
    } else {
      m.inner.quaternion.copy(camQ).premultiply(state.parentInv);
    }
  }
}

// ─── raycasting: the app's real path (three.js) ────────────────────────────
function threeHitAt(px, py) {
  if (!state.mesh3 || !state.cam3) return null;
  state.mesh3.updateMatrixWorld(true);
  const ndcX = (px / state.viewport.width) * 2 - 1;
  const ndcY = -((py / state.viewport.height) * 2) + 1;
  state.raycaster.setFromCamera(new THREE.Vector2(ndcX, ndcY), state.cam3);
  state.lastRay = {
    origin: state.raycaster.ray.origin.clone(),
    direction: state.raycaster.ray.direction.clone(),
  };
  // Material side is DoubleSide in the app (Viewport.tsx:2589/2591).
  const hits = state.raycaster.intersectObject(state.mesh3, false);
  if (!hits.length) return null;
  const h = hits[0];
  // getLocalHit: meshRef.current.worldToLocal(h.point.clone()) — Viewport.tsx:1457
  const localV = state.mesh3.worldToLocal(h.point.clone());
  return {
    faceIndex: h.faceIndex ?? -1,
    world: [h.point.x, h.point.y, h.point.z],
    local: [localV.x, localV.y, localV.z],
    distance: h.distance,
  };
}

// ─── one click, all six strategies ─────────────────────────────────────────
function evaluateAt(px, py, { intended = null, hitOverride = null } = {}) {
  const hit = hitOverride ?? threeHitAt(px, py);
  const seedsLocal = state.seeds.map((s) => [s[0], s[1], s[2]]);
  const ev = C.evaluateFromHit({
    clickPx: [px, py],
    cam: state.cam,
    seedsLocal,
    markerSize: state.sizes.size,
    intended,
    hit,
  });
  return ev;
}

function handleLeftClick(px, py) {
  if (!state.model) return;
  const hit = threeHitAt(px, py);
  let ev;

  if (state.mode === "sow") {
    if (!hit) {
      ev = evaluateAt(px, py, { hitOverride: null });
      setLastEval(ev);
      toast("click missed the mesh — the sapling glyph above the surface is not raycastable (only the mesh is)", "bad");
      renderClickTable(ev);
      pushLog("miss", `sow click px=(${fmt(px, 0)}, ${fmt(py, 0)}) hit nothing → no seed (getLocalHit returns null)`);
      return;
    }
    const centroid = C.faceCentroid(state.model, hit.faceIndex);
    state.seeds.push(centroid);
    rebuildMarkers();
    ev = evaluateAt(px, py, { intended: state.seeds.length - 1, hitOverride: hit });
    setLastEval(ev);
    renderClickTable(ev);
    pushLog("sow", `sow px=(${fmt(px, 0)}, ${fmt(py, 0)}) face ${hit.faceIndex} → seed #${state.seeds.length - 1} at local ${triple(centroid, 3)}`);
    renderStageNote();
    return;
  }

  // ── ERASE mode — the MATCHER decides what actually happens ──
  const ev0 = evaluateAt(px, py, { hitOverride: hit });
  const match0 = eraseMatch(ev0);
  const focus = match0.index >= 0 ? match0.index : match0.row && match0.row.nearestSeed != null ? match0.row.nearestSeed : null;
  ev = evaluateAt(px, py, { intended: focus, hitOverride: hit });
  setLastEval(ev);
  renderClickTable(ev);

  const m = match0.row;
  const headPx = m ? (m.unit === "px" ? m.value : m.value * state.cam.zoom) : NaN;
  const thrPx = m ? (m.unit === "px" ? m.threshold : m.threshold * state.cam.zoom) : NaN;
  const matcherLabel = state.matcher === "s2" ? "S2 (corrected)" : "S5 (HEAD)";

  if (match0.index >= 0) {
    if (state.mutate) {
      const removed = state.seeds.splice(match0.index, 1)[0];
      rebuildMarkers();
      pushLog(
        "erase",
        `erase ${matcherLabel} px=(${fmt(px, 0)}, ${fmt(py, 0)}) → seed #${match0.index} |Δ|=${fmt(m.value, m.unit === "px" ? 1 : 4)} ${m.unit} ≤ ${fmt(m.threshold, m.unit === "px" ? 1 : 4)} → REMOVED (local ${triple(removed, 2)})`
      );
      toast(
        state.matcher === "s2"
          ? `S2 (corrected) erased seed #${match0.index} — pointer was ${fmt(headPx, 1)} px away, tolerance ${fmt(thrPx, 1)} px`
          : `S5 (HEAD) erased seed #${match0.index} — you landed within ${fmt(thrPx, 1)} px of the centroid`,
        "good"
      );
    } else {
      pushLog(
        "erase-would",
        `erase ${matcherLabel} WOULD remove seed #${match0.index} (|Δ|=${fmt(m.value, m.unit === "px" ? 1 : 4)} ${m.unit} ≤ ${fmt(m.threshold, m.unit === "px" ? 1 : 4)}); mutation is off`
      );
    }
  } else {
    pushLog(
      hit ? "noerase" : "miss",
      `erase ${matcherLabel} px=(${fmt(px, 0)}, ${fmt(py, 0)}) → ` +
        (hit
          ? `nearest seed #${m.nearestSeed}: |Δ|=${fmt(m.value, m.unit === "px" ? 1 : 4)} ${m.unit} > tolerance ${fmt(m.threshold, m.unit === "px" ? 1 : 4)} → nothing erased`
          : `getLocalHit() = null` + (state.matcher === "s5" ? "; the app returns before eraseNearWorldPoint() → nothing erased" : ""))
    );
    if (state.matcher === "s5") {
      toast(
        hit
          ? `S5 (HEAD) missed: ${fmt(m.value, 3)} wu ≥ tolerance ${fmt(m.threshold, 4)} wu ≈ ${fmt(thrPx, 1)} px of aim. Switch the matcher to “S2 · fixed”.`
          : `No mesh under the cursor → getLocalHit() is null, so HEAD cannot erase. With “S2 · fixed” this click still erases (pointer is ${fmt(headPx, 1)} px from the marker).`,
        "bad"
      );
    } else {
      toast(`S2 (corrected) missed too: nearest marker is ${fmt(headPx, 1)} px from the pointer (> 32 px).`, "bad");
    }
  }
  renderStageNote();
  crossCheck(px, py, hit);
}

function setLastEval(ev) {
  state.lastEval = ev;
}

/** Which strategy the lab's eraser is currently using. */
function eraseMatch(ev) {
  const id = state.matcher === "s2" ? "S2" : "S5";
  const row = ev.rows.find((r) => r.id === id);
  return { id, row, index: row && row.pass && row.nearestSeed != null ? row.nearestSeed : -1 };
}

/** HUD plumbing for the matcher (obvious in the stage bar, the card and the facts). */
function applyMatcher() {
  if (el.matcherGroup) {
    for (const b of el.matcherGroup.querySelectorAll("button")) b.classList.toggle("on", b.dataset.matcher === state.matcher);
  }
  if (el.matcherChoices) {
    for (const inp of el.matcherChoices.querySelectorAll("input[name=matcher]")) inp.checked = inp.value === state.matcher;
  }
  renderStageNote(); // refreshMatcherTexts() runs there
  renderFacts();
}

/** Numbers in the matcher chrome depend on the live zoom, so refresh them often. */
function refreshMatcherTexts() {
  const s2 = state.matcher === "s2";
  const thrPx = state.sizes.thresh * (state.cam ? state.cam.zoom : 1);
  if (el.matcherChip) {
    el.matcherChip.className = `chip ${s2 ? "chip--green" : "chip--red"}`;
    el.matcherChip.textContent = s2
      ? "S2: 32 px tolerance · reliable"
      : `S5 tolerance ≈ ${fmt(thrPx, 2)} px · aim-sensitive`;
  }
  if (el.s5Desc) {
    el.s5Desc.textContent = `worldToLocal(hit) vs local seed · threshold 2.5×size = ${fmt(state.sizes.thresh, 4)} wu ≈ ${fmt(thrPx, 2)} px. Clicks that miss the surface do nothing, and the aim must land inside that radius.`;
  }
  if (el.matcherHint) {
    el.matcherHint.textContent = s2
      ? "S2 corrected: click anywhere on a red marker (or its glyph) within 32 px — it always erases, even off-surface."
      : `S5 at HEAD: you must land inside ${fmt(thrPx, 2)} px of the centroid AND hit the mesh. Try a normal aim, then switch to “S2 · fixed”.`;
  }
}

// ─── cross-check: core math vs the real three.js engine ────────────────────
function crossCheck(px, py, threeHit = null) {
  if (!state.cam || !state.cam3) return;
  const cam = state.cam;
  const th = threeHit ?? threeHitAt(px, py);
  const coreRay = C.rayFromScreen(px, py, cam);

  const dRayO = state.lastRay
    ? C.dist3(coreRay.origin, [state.lastRay.origin.x, state.lastRay.origin.y, state.lastRay.origin.z])
    : NaN;
  const dRayD = state.lastRay
    ? C.dist3(coreRay.direction, [
        state.lastRay.direction.x,
        state.lastRay.direction.y,
        state.lastRay.direction.z,
      ])
    : NaN;

  const m = state.cam3.matrixWorld.elements;
  const dBasis = Math.max(
    C.dist3(cam.xAxis, [m[0], m[1], m[2]]),
    C.dist3(cam.yAxis, [m[4], m[5], m[6]]),
    C.dist3(cam.zAxis, [m[8], m[9], m[10]])
  );

  let dProj = 0;
  const v = new THREE.Vector3();
  for (const s of state.seeds) {
    const w = C.localToWorld(s);
    v.set(w[0], w[1], w[2]).project(state.cam3);
    const threePx = [(v.x * 0.5 + 0.5) * cam.width, (1 - (v.y * 0.5 + 0.5)) * cam.height];
    const corePx = C.projectWorld(w, cam);
    dProj = Math.max(dProj, Math.hypot(threePx[0] - corePx[0], threePx[1] - corePx[1]));
  }

  let dHit = null;
  if (state.model.faceCount <= 200000) {
    const ch = C.raycastMesh(coreRay, state.model);
    if (th && ch) dHit = Math.max(C.dist3(th.world, ch.world), C.dist3(th.local, ch.local));
    else if (!th && !ch) dHit = 0;
    else dHit = Infinity;
  }

  state.xcheck = { dRayO, dRayD, dBasis, dProj, dHit, px, py, engine: "three r170" };
  renderXCheck();
}

function renderXCheck() {
  const x = state.xcheck;
  if (!x) return;
  const worst = Math.max(x.dRayO, x.dRayD, x.dBasis, x.dProj, x.dHit ?? 0);
  const ok = worst < 1e-6;
  const bits = [
    `ray origin Δ ${x.dRayO.toExponential(1)}`,
    `ray dir Δ ${x.dRayD.toExponential(1)}`,
    `camera basis Δ ${x.dBasis.toExponential(1)}`,
    `${state.seeds.length} seed projections Δ ${x.dProj.toExponential(1)} px`,
    x.dHit == null ? "hit Δ skipped (mesh too large)" : `hit point Δ ${x.dHit.toExponential(1)} wu`,
  ];
  el.xcheck.innerHTML = `<span class="v ${ok ? "v--pass" : "v--fail"}">${ok ? "MATCH" : "MISMATCH"}</span> core.mjs ↔ three.js: ${bits.join(" · ")}`;
}

// ─── overlay (crosshairs) ──────────────────────────────────────────────────
const OV_COLORS = {
  green: "#3ddc84",
  magenta: "#ff2bd6",
  white: "#ffffff",
  red: "#ff4d4f",
  cyan: "#22d3ee",
  amber: "#fbbf24",
};

function ctxOverlay() {
  const ctx = el.overlay.getContext("2d");
  ctx.setTransform(state.dpr, 0, 0, state.dpr, 0, 0);
  return ctx;
}

function drawOverlay() {
  const ctx = ctxOverlay();
  const { width: W, height: H } = state.viewport;
  ctx.clearRect(0, 0, W, H);
  if (!state.overlay || !state.cam) return;

  const focusSeed = state.lastEval?.overlay?.seedLocal ?? state.seeds[0] ?? null;
  if (focusSeed) {
    drawLayer(ctx, state.cam, 0, 0, { compact: false, focusSeed });
    if (state.inset) drawInset(ctx, focusSeed);
  }
}

function drawLayer(ctx, cam, ox, oy, { compact, focusSeed }) {
  const P = (p) => {
    const q = C.projectWorld(p, cam);
    return [q[0] + ox, q[1] + oy];
  };
  const seedWorld = C.localToWorld(focusSeed);
  const rendered = P(seedWorld);
  const phantom = P([focusSeed[0], focusSeed[1], focusSeed[2]]);
  const zoom = cam.zoom;
  const eraseR = Math.max(1.5, state.sizes.thresh * zoom);
  const glyphR = C.glyphPoints(state.sizes.size).maxRadius * zoom;

  // all markers: a dot at each rendered position
  for (const s of state.seeds) {
    const q = P(C.localToWorld(s));
    ctx.beginPath();
    ctx.arc(q[0], q[1], compact ? 2.5 : 2, 0, Math.PI * 2);
    ctx.fillStyle = OV_COLORS.green;
    ctx.fill();
  }

  // erase radius (what S5 compares against) — the target really is this small
  ctx.beginPath();
  ctx.arc(rendered[0], rendered[1], eraseR, 0, Math.PI * 2);
  ctx.strokeStyle = "rgba(61,220,132,0.9)";
  ctx.lineWidth = 1.2;
  ctx.setLineDash([3, 3]);
  ctx.stroke();
  ctx.setLineDash([]);

  // S2/S4 screen tolerance (32 px) — what the corrected match allows
  if (!compact) {
    ctx.beginPath();
    ctx.arc(rendered[0], rendered[1], C.SCREEN_THRESHOLD_PX, 0, Math.PI * 2);
    ctx.strokeStyle = "rgba(34,211,238,0.55)";
    ctx.lineWidth = 1;
    ctx.setLineDash([2, 4]);
    ctx.stroke();
    ctx.setLineDash([]);
    label(
      ctx,
      rendered[0] + C.SCREEN_THRESHOLD_PX * 0.72,
      rendered[1] + C.SCREEN_THRESHOLD_PX + 6,
      `S2 radius ${fmt(C.SCREEN_THRESHOLD_PX, 0)} px`,
      OV_COLORS.cyan
    );
  }

  // glyph footprint: as drawn (bug) vs corrected
  const g = C.glyphPoints(state.sizes.size);
  const footprint = (mode) => {
    const pts = {};
    for (const k of ["center", "stemTop", "leafL", "leafR"]) {
      pts[k] = P(C.add3(seedWorld, C.glyphPointOffset(cam, g[k], mode)));
    }
    return pts;
  };
  const bugPts = footprint("bug");
  const okPts = footprint("correct");
  const poly = (pts, color, dash, width) => {
    ctx.beginPath();
    ctx.moveTo(pts.leafL[0], pts.leafL[1]);
    ctx.lineTo(pts.stemTop[0], pts.stemTop[1]);
    ctx.lineTo(pts.leafR[0], pts.leafR[1]);
    ctx.moveTo(pts.center[0], pts.center[1]);
    ctx.lineTo(pts.stemTop[0], pts.stemTop[1]);
    ctx.strokeStyle = color;
    ctx.lineWidth = width;
    ctx.setLineDash(dash);
    ctx.stroke();
    ctx.setLineDash([]);
    for (const k of ["leafL", "leafR"]) {
      ctx.beginPath();
      ctx.arc(pts[k][0], pts[k][1], Math.max(1.5, 0.385 * state.sizes.size * zoom), 0, Math.PI * 2);
      ctx.strokeStyle = color;
      ctx.lineWidth = width;
      ctx.stroke();
    }
  };
  poly(okPts, "rgba(34,211,238,0.55)", [2, 3], 1);
  poly(bugPts, OV_COLORS.red, [], 1.3);

  // rendered marker (= S2 position) crosshair
  cross(ctx, rendered[0], rendered[1], OV_COLORS.green, compact ? 6 : 9, compact ? 1 : 1.4);
  label(ctx, rendered[0] + 12, rendered[1] - 12, compact ? "S2" : "S2 rendered marker", OV_COLORS.green);

  // S1 phantom (local coordinates used as world)
  const inView = phantom[0] > 8 && phantom[0] < cam.width - 8 && phantom[1] > 8 && phantom[1] < cam.height - 8;
  if (inView) {
    cross(ctx, phantom[0], phantom[1], OV_COLORS.magenta, compact ? 6 : 10, compact ? 1 : 1.4);
    label(ctx, phantom[0] + 12, phantom[1] + 16, compact ? "S1" : "S1 phantom (local-as-world)", OV_COLORS.magenta);
  } else {
    // clamp to the border with an arrow so the reader can see it is off-screen
    const cx = Math.max(14, Math.min(cam.width - 14, phantom[0]));
    const cy = Math.max(14, Math.min(cam.height - 14, phantom[1]));
    cross(ctx, cx, cy, OV_COLORS.magenta, compact ? 5 : 8, 1.2);
    if (!compact) {
      const d = Math.hypot(phantom[0] - rendered[0], phantom[1] - rendered[1]);
      label(ctx, cx + 10, cy + 16, `S1 off-screen (Δ ${fmt(d, 0)} px from marker)`, OV_COLORS.magenta);
    }
  }

  // rendered → phantom line with the px distance (this is the whole bug in one line)
  if (!compact && (Math.abs(phantom[0] - rendered[0]) > 0.5 || Math.abs(phantom[1] - rendered[1]) > 0.5)) {
    ctx.beginPath();
    ctx.moveTo(rendered[0], rendered[1]);
    ctx.lineTo(phantom[0], phantom[1]);
    ctx.strokeStyle = "rgba(255,43,214,0.55)";
    ctx.lineWidth = 1;
    ctx.setLineDash([5, 4]);
    ctx.stroke();
    ctx.setLineDash([]);
    const d = Math.hypot(phantom[0] - rendered[0], phantom[1] - rendered[1]);
    label(
      ctx,
      (rendered[0] + phantom[0]) / 2 + 6,
      (rendered[1] + phantom[1]) / 2 + 14,
      `S1 is ${fmt(d, 1)} px from the drawn marker (screen threshold 32 px)`,
      OV_COLORS.amber
    );
  }

  // the click point
  const snap = state.lastEval;
  if (snap && !compact) {
    // world point of the click at the seed's depth (ortho: a pure screen offset)
    const dx = (snap.clickPx[0] - C.projectWorld(C.localToWorld(focusSeed), state.cam)[0]) / state.cam.zoom;
    const dy = (snap.clickPx[1] - C.projectWorld(C.localToWorld(focusSeed), state.cam)[1]) / state.cam.zoom;
    const clickWorld = C.add3(
      C.add3(seedWorld, C.scale3(state.cam.xAxis, dx)),
      C.scale3(state.cam.yAxis, -dy)
    );
    const cp = P(clickWorld);
    cross(ctx, cp[0], cp[1], OV_COLORS.white, 8, 1.2);
    label(ctx, cp[0] + 12, cp[1] + 16, "click", OV_COLORS.white);
  }

  if (!compact) {
    label(
      ctx,
      14,
      cam.height - 60,
      `erase radius ${fmt(state.sizes.thresh * zoom, 2)} px · glyph extent ≈ ${fmt(glyphR, 2)} px (true scale)`,
      OV_COLORS.green
    );
  }
}

function drawInset(ctx, focusSeed) {
  const { width: W, height: H } = state.viewport;
  const iw = 200;
  const ih = 150;
  const margin = 14;
  if (W < 520 || H < 320) return;
  const rect = { x: W - iw - margin, y: margin, w: iw, h: ih };
  state.insetRect = rect;

  // frame + caption (the WebGL inset itself is rendered in renderScene())
  ctx.save();
  ctx.beginPath();
  ctx.rect(rect.x, rect.y, rect.w, rect.h);
  ctx.strokeStyle = "rgba(255,255,255,0.35)";
  ctx.lineWidth = 1;
  ctx.stroke();
  ctx.clip();

  const cam = state.insetCam;
  if (cam) {
    ctx.translate(rect.x, rect.y);
    drawLayer(ctx, cam, 0, 0, { compact: true, focusSeed });
    ctx.setTransform(state.dpr, 0, 0, state.dpr, 0, 0);
  }
  ctx.restore();

  label(ctx, rect.x + 6, rect.y + 12, `${INSET_MAG}× magnifier · true marker size`, OV_COLORS.cyan);
}

function cross(ctx, x, y, color, arm = 8, width = 1.2) {
  ctx.beginPath();
  ctx.moveTo(x - arm, y);
  ctx.lineTo(x + arm, y);
  ctx.moveTo(x, y - arm);
  ctx.lineTo(x, y + arm);
  ctx.strokeStyle = color;
  ctx.lineWidth = width;
  ctx.stroke();
  ctx.beginPath();
  ctx.arc(x, y, Math.max(1.5, arm * 0.42), 0, Math.PI * 2);
  ctx.strokeStyle = color;
  ctx.lineWidth = width;
  ctx.stroke();
}

function label(ctx, x, y, text, color) {
  ctx.font = "10.5px ui-monospace, SFMono-Regular, Menlo, monospace";
  const w = ctx.measureText(text).width + 8;
  const h = 14;
  const px = Math.max(2, Math.min(x, state.viewport.width - w - 2));
  const py = Math.max(2, Math.min(y - 11, state.viewport.height - h - 2));
  ctx.fillStyle = "rgba(8,10,14,0.78)";
  ctx.fillRect(px, py, w, h);
  ctx.strokeStyle = "rgba(255,255,255,0.12)";
  ctx.lineWidth = 1;
  ctx.strokeRect(px + 0.5, py + 0.5, w - 1, h - 1);
  ctx.fillStyle = color;
  ctx.fillText(text, px + 4, py + 10.5);
}

// ─── rendering ─────────────────────────────────────────────────────────────
function renderScene() {
  const gl = state.gl;
  const { width: W, height: H } = state.viewport;
  gl.setViewport(0, 0, W, H);
  gl.setScissorTest(false);
  gl.setClearColor(0x2a2a2a, 1);
  gl.render(state.scene, state.cam3);

  if (!state.inset || !state.lastEval?.overlay?.seedLocal) {
    state.insetCam = null;
    return;
  }
  const seedWorld = C.localToWorld(state.lastEval.overlay.seedLocal);
  const cam = state.cam;
  const dir = cam.zAxis;
  const pos = C.add3(seedWorld, C.scale3(dir, cam.viewDist));
  const iw = 200;
  const ih = 150;
  const rect = { x: W - iw - 14, y: 14, w: iw, h: ih };
  if (W < 520 || H < 320) {
    state.insetCam = null;
    return;
  }

  // The magnifier camera: same orientation, centred on the seed, zoom ×8.
  const icam = C.poseCamera({
    position: pos,
    target: seedWorld,
    zoom: cam.zoom * INSET_MAG,
    width: iw,
    height: ih,
    near: cam.near,
    far: cam.far,
  });
  state.insetCam = icam;
  if (!state.insetCam3) state.insetCam3 = new THREE.OrthographicCamera(-iw / 2, iw / 2, ih / 2, -ih / 2, cam.near, cam.far);
  const c3 = state.insetCam3;
  c3.left = -iw / 2;
  c3.right = iw / 2;
  c3.top = ih / 2;
  c3.bottom = -ih / 2;
  c3.near = cam.near;
  c3.far = cam.far;
  c3.zoom = cam.zoom * INSET_MAG;
  c3.up.set(0, 1, 0);
  c3.position.set(pos[0], pos[1], pos[2]);
  c3.lookAt(seedWorld[0], seedWorld[1], seedWorld[2]);
  c3.updateProjectionMatrix();
  c3.updateMatrixWorld(true);

  const glY = H - (rect.y + rect.h);
  gl.setViewport(rect.x, glY, rect.w, rect.h);
  gl.setScissorTest(true);
  gl.setScissor(rect.x, glY, rect.w, rect.h);
  gl.setClearColor(0x0d1014, 1);
  gl.clear(true, true, false);
  gl.render(state.scene, c3);
  gl.setScissorTest(false);
  gl.setViewport(0, 0, W, H);
  gl.setClearColor(0x2a2a2a, 1);
  state.insetRect = rect;
}

function tick() {
  requestAnimationFrame(tick);
  if (!state.cam3) return;
  updateBillboard();
  renderScene();
  drawOverlay();
}

// ─── HUD: facts ────────────────────────────────────────────────────────────
function renderFacts() {
  if (!state.model || !state.cam) return;
  const cam = state.cam;
  const fit = state.fit ?? cam;
  let tilt = null;
  if (state.markers.length) {
    const q = new THREE.Quaternion();
    state.scene.updateMatrixWorld(true);
    state.markers[0].inner.getWorldQuaternion(q);
    const dot = Math.abs(q.dot(state.cam3.quaternion));
    tilt = (2 * Math.acos(Math.min(1, dot)) * 180) / Math.PI;
  }
  const glyphRadiusPx = C.glyphPoints(state.sizes.size).maxRadius * cam.zoom;
  const facts = [
    ["model", `${state.model.label}`],
    ["faces", `${state.model.faceCount.toLocaleString()}`],
    ["viewport", `${cam.width}×${cam.height} px`],
    ["zoom (fit)", `${fmt(fit.zoom ?? cam.zoom, 3)}`],
    ["worldPerPx", `${fmt(cam.worldPerPx, 5)}`],
    ["seedMarkerSize", `${fmt(state.sizes.size, 5)} wu`],
    ["erase radius", `${fmt(state.sizes.thresh, 4)} wu = ${fmt(state.sizes.thresh * cam.zoom, 2)} px`],
    ["glyph extent", `≈ ${fmt(glyphRadiusPx, 2)} px from the seed`],
    ["erase matching", state.matcher === "s2" ? "S2 · corrected (32 px)" : `S5 · HEAD (${fmt(state.sizes.thresh * cam.zoom, 1)} px)`],
    ["glyph tilt", tilt == null ? "—" : `${fmt(tilt, 3)}° off camera`],
    ["seeds", `${state.seeds.length}`],
  ];
  el.facts.innerHTML = facts
    .map(([k, v]) => {
      const warn =
        (k === "glyph tilt" && tilt != null && tilt > 1) ||
        (k === "erase matching" && state.matcher === "s5");
      return `<div><dt>${esc(k)}</dt><dd class="${warn ? "warn" : ""}">${esc(v)}</dd></div>`;
    })
    .join("");
}

function renderStageNote() {
  const cam = state.cam;
  refreshMatcherTexts();
  el.stageNote.textContent =
    `mode=${state.mode} · seeds=${state.seeds.length} · zoom=${fmt(cam.zoom, 2)} · billboard=${state.billboard}` +
    (state.smear.attempts ? ` · smear ${state.smear.attempts} fired/${state.smear.hits} matched` : "");
}

// ─── HUD: per-click table ──────────────────────────────────────────────────
function renderClickTable(ev) {
  const tbody = el.tblClick.querySelector("tbody");
  if (!ev) {
    tbody.innerHTML = '<tr class="empty"><td colspan="5">no click yet</td></tr>';
    return;
  }
  const hit = ev.hit;
  el.clickMeta.textContent = `px=(${fmt(ev.clickPx[0], 0)}, ${fmt(ev.clickPx[1], 0)}) · face ${hit ? hit.faceIndex : "MISS"} · world ${triple(hit?.world)} · local ${triple(hit?.local)}`;

  tbody.innerHTML = ev.rows
    .map((r) => {
      const verdict = r.unavailable
        ? '<span class="v v--na">N/A</span>'
        : r.strict
        ? '<span class="v v--pass">PASS</span>'
        : r.pass
        ? '<span class="v v--warn">PASS?</span>'
        : '<span class="v v--fail">FAIL</span>';
      const nearest = r.nearestSeed == null ? "—" : `#${r.nearestSeed}`;
      return `<tr>
        <td><span class="sid">${r.id}</span> ${esc(r.label)}<span class="sub">${esc(r.source)}</span></td>
        <td class="num val">${r.value == null ? "—" : fmt(r.value, r.unit === "px" ? 1 : 4)} <span class="muted">${r.unit}</span></td>
        <td class="num val">${fmt(r.threshold, r.unit === "px" ? 1 : 4)}</td>
        <td class="num val">${nearest}</td>
        <td>${verdict}</td>
      </tr>`;
    })
    .join("");

  const s5 = ev.rows.find((r) => r.id === "S5");
  const s6 = ev.rows.find((r) => r.id === "S6");
  const s2 = ev.rows.find((r) => r.id === "S2");
  const tolPx = s5.threshold * state.cam.zoom;
  const glyphPx = C.glyphPoints(state.sizes.size).maxRadius * state.cam.zoom;
  const active = state.matcher === "s2" ? s2 : s5;
  const activeId = state.matcher === "s2" ? "S2 (corrected)" : "S5 (HEAD)";
  const activeLine = active.unavailable
    ? "cannot even run (no surface hit)"
    : active.strict
    ? `ERASES seed #${active.nearestSeed}`
    : `does nothing (nearest #${active.nearestSeed ?? "—"} at ${fmt(active.value, active.unit === "px" ? 1 : 4)} ${active.unit})`;
  el.clickNote.innerHTML =
    `<b class="mono">erase matching = ${activeId}</b> → ${activeLine}. ` +
    (hit
      ? `S5 tolerance ${fmt(s5.threshold, 4)} wu ≈ ${fmt(tolPx, 2)} px vs S2/S4 ${fmt(
          C.SCREEN_THRESHOLD_PX,
          0
        )} px; sapling ≈${fmt(glyphPx, 2)} px, billboard ${fmt(C.glyphWorldFrame(state.cam, state.billboard).tiltDeg, 0)}° off camera.`
      : `<b class="mono">No mesh under this pixel</b> → <code>getLocalHit()</code> is null, so HEAD's S5 path returns before
         <code>eraseNearWorldPoint()</code> (markers are not raycastable). S2 needs no hit: pointer is ${fmt(s2.value, 1)} px away.`);
}

function pushLog(kind, text) {
  state.lastLog.unshift({ kind, text, t: new Date() });
  state.lastLog = state.lastLog.slice(0, 9);
  el.clickLog.innerHTML = state.lastLog
    .map((l) => {
      const cls = kind === "erase" ? "good" : kind === "smear" ? (l.text.includes("MATCH") ? "good" : "") : "bad";
      return `<li><span class="${cls}">${esc(l.text)}</span></li>`;
    })
    .join("");
}

function renderSmear() {
  const s = state.smear;
  const s2 = state.matcher === "s2";
  if (!s.attempts) {
    el.smearLine.innerHTML = s2
      ? "Corrected smear: every pointermove compares the <b>world</b> seed with the picking ray (32 px). Dragging across markers wipes them."
      : "In erase mode, press the left button on a marker and drag: every pointermove calls " +
        "<code>eraseNearWorldPoint(worldHit)</code> — the S6 path. Nothing matches, because a WORLD hit is compared " +
        "with MODEL-LOCAL seeds. The counters will show it.";
    return;
  }
  const last = s.last
    ? `nearest seed #${s.last.best}: ${fmt(s.last.value, s.last.unit === "px" ? 1 : 3)} ${s.last.unit} vs tolerance ${fmt(
        s.last.thresh,
        s.last.unit === "px" ? 0 : 3
      )} ${s.last.unit}`
    : "no mesh under the cursor";
  el.smearLine.innerHTML =
    `<b class="mono">${s.attempts}</b> smear sample(s) fired with ${s2 ? "S2/S4 (corrected)" : "S6 (HEAD)"} · ` +
    `<b class="mono ${s.hits ? "good" : "bad"}">${s.hits}</b> matched · ${esc(last)}`;
}

// ─── self-test ─────────────────────────────────────────────────────────────
async function runSelfTest(auto = false) {
  if (!state.model) return;
  el.btnSelfTest.disabled = true;
  setStatus("busy", auto ? "running (automatic on load)…" : "running…");
  await nextFrame();
  try {
    // Faithful: the self-test starts from the fitted camera.
    resetView();
    state.seeds = [];
    state.lastEval = null;
    state.xcheck = null;
    rebuildMarkers();

    const count = seedCountValue();
    const placed = autoPlace(count);
    if (!placed.length) {
      setStatus("fail", "self-test FAIL — could not place any seed that is cleanly clickable in this view (try Reset view)");
      return;
    }
    // Two aim scenarios: centred, and 4 px off (human aim).
    const scenarios = C.evaluateScenarios({
      mesh: state.model,
      cam: state.cam,
      seeds: placed,
      markerSize: state.sizes.size,
      raycastAt: threeHitAt,
    });
    const firstClick = scenarios[0].clicks[0];
    if (firstClick) {
      state.lastEval = firstClick.evaluation;
      renderClickTable(firstClick.evaluation);
      crossCheck(firstClick.clickPx[0], firstClick.clickPx[1]);
    }

    const report = C.buildReport({
      mesh: state.model,
      cam: state.cam,
      viewport: state.viewport,
      seeds: placed,
      scenarios,
    });
    report.engine = "three.js r170 Raycaster (the page's real path)";
    report.crossCheck = state.xcheck;
    report.auto = auto;
    state.selfTest = report;
    window.__forensicReport = report;
    renderSelfTest(report);
    renderFacts();
    renderStageNote();
    renderSmear();
  } catch (err) {
    console.error(err);
    setStatus("fail", `self-test crashed: ${err && err.message ? err.message : String(err)}`);
  } finally {
    el.btnSelfTest.disabled = false;
  }
}

function setStatus(kind, text) {
  el.stStatus.className = `status ${kind === "pass" ? "pass" : kind === "fail" ? "fail" : kind === "busy" ? "busy" : ""}`;
  el.stStatus.textContent = text;
}

/** Compact labels for the two-scenario table only (the per-click table keeps the
 *  full strategy names — that one is locked to the six-strategy spec). */
const SHORT = {
  S1: "project(local seed)",
  S2: "project(world seed)",
  S3: "ray→local seed",
  S4: "ray→world seed",
  S5: "worldToLocal(hit)",
  S6: "hit→local seed",
};
const SHORT_SRC = {
  S1: "Viewport.tsx:1939 R6/R7",
  S2: "corrected R6/R7",
  S3: "Viewport.tsx:1950 R8-first",
  S4: "corrected R8-first",
  S5: "Viewport.tsx:1991 R8-final click",
  S6: "Viewport.tsx:2125 R8-final smear",
};

function renderSelfTest(report) {
  const checks = report.scenarios.reduce((n, sc) => n + sc.aggregate.length, 0);
  const okAll = report.expectedVsActual.allOk;
  const okCount = report.scenarios.reduce((n, sc) => n + sc.aggregate.filter((a) => a.ok).length, 0);
  setStatus(
    okAll ? "pass" : "fail",
    `${okAll ? "SELF-TEST PASS" : "SELF-TEST FAIL"} — ${okCount}/${checks} checks over ${report.scenarios.length} scenarios` +
      ` · model=${report.model.id} (${report.model.faceCount.toLocaleString()} faces)` +
      ` · viewport=${report.viewport.width}×${report.viewport.height} · zoom=${fmt(report.camera.zoom, 3)}` +
      ` · S5 tolerance ${fmt(report.sizes.eraseThresholdWorld, 4)} wu ≈ ${fmt(report.sizes.s5TolerancePx, 2)} px | S2/S4 32 px` +
      ` · seeds=${report.seeds.length} · glyph tilt ${fmt(report.sizes.glyphTiltBugDeg, 1)}°` +
      (okAll ? "" : ` · ${report.expectedVsActual.mismatches.join(" | ")}`) +
      ` · report → window.__forensicReport`
  );

  el.stTables.innerHTML = report.scenarios
    .map((sc) => {
      const rows = sc.aggregate
        .map((a) => {
          const tol =
            a.unit === "px"
              ? `${fmt(a.threshold, 1)} px`
              : `${fmt(a.threshold, 4)} wu ≈ ${fmt(report.sizes.s5TolerancePx, 2)} px`;
          const evidence =
            (a.expected === "FAIL"
              ? `${a.wrongSeed}/${a.totalClicks} wrong seed · best ${fmt(a.minValue, a.unit === "px" ? 1 : 4)} ${a.unit}`
              : `worst ${fmt(a.maxValue, a.unit === "px" ? 1 : 4)} ${a.unit}`) +
            (a.expectedNote ? `<span class="note">${esc(a.expectedNote)}</span>` : "");
          return `<tr>
            <td><span class="sid">${a.id}</span> ${esc(SHORT[a.id] || a.label)}</td>
            <td class="num tol">${tol}</td>
            <td class="num"><span class="v ${a.expected === "PASS" ? "v--pass" : "v--fail"}">${a.expected}</span></td>
            <td class="num"><span class="v ${vcls(a.actual)}">${a.actual}</span></td>
            <td class="num">${a.passedClicks}/${a.totalClicks}</td>
            <td class="ev">${evidence}</td>
          </tr>`;
        })
        .join("");
      return `<div class="scen-block">
        <div class="scen-block__head">
          <b>scenario ${esc(sc.id)}</b>
          <span class="muted">${esc(sc.label)} · offset (${sc.offsetPx[0]}, ${sc.offsetPx[1]}) px</span>
          <span class="v ${sc.allOk ? "v--pass" : "v--fail"}">${sc.allOk ? "as expected" : "MISMATCH"}</span>
        </div>
        <table class="tbl">
          <thead><tr><th>Strategy</th><th class="num">Tolerance</th><th class="num">Expected</th><th class="num">Actual</th><th class="num">Hits</th><th>Evidence</th></tr></thead>
          <tbody>${rows}</tbody>
        </table>
      </div>`;
    })
    .join("");

  // per-click detail, both scenarios
  el.stDetail.innerHTML = report.scenarios
    .map((sc) => {
      const head = `<h3>scenario ${esc(sc.id)} — ${esc(sc.label)}</h3>`;
      const clicks = sc.clicks
        .map((c) => {
          const rowHtml = c.rows
            .map(
              (r) => `<tr>
              <td><span class="sid">${r.id}</span></td>
              <td class="num val">${r.value == null ? "—" : fmt(r.value, r.unit === "px" ? 1 : 4)} ${r.unit}</td>
              <td class="num val">${r.toleranceText ? esc(r.toleranceText) : fmt(r.threshold, 3)}</td>
              <td class="num val">#${r.nearestSeed ?? "—"} / aimed #${r.intended}</td>
              <td><span class="v ${r.strict ? "v--pass" : r.pass ? "v--warn" : "v--fail"}">${r.strict ? "PASS" : r.pass ? "PASS?" : "FAIL"}</span></td>
            </tr>`
            )
            .join("");
          return `<div class="clickblock">
            <div class="ev">click ${c.seedIndex + 1}/${sc.clicks.length} · px=(${fmt(c.clickPx[0], 0)}, ${fmt(c.clickPx[1], 0)}) ·
              hit face ${c.hit ? c.hit.faceIndex : "MISS"} · local ${triple(c.hit?.local, 2)}</div>
            <table class="tbl"><thead><tr><th>Strategy</th><th class="num">Value</th><th class="num">Tolerance</th><th class="num">Nearest / aimed</th><th>Verdict</th></tr></thead><tbody>${rowHtml}</tbody></table>
          </div>`;
        })
        .join("");
      return head + clicks;
    })
    .join("");
  el.stDetails.open = !okAll;
}

// ─── auto-place / seeds ────────────────────────────────────────────────────
function seedCountValue() {
  const n = parseInt(el.seedCount.value, 10);
  return Math.max(1, Math.min(12, Number.isFinite(n) ? n : 5));
}

function autoPlace(count) {
  if (!state.model || !state.cam) return [];
  const placed = C.autoPlaceSeeds({
    mesh: state.model,
    cam: state.cam,
    count,
    markerSize: state.sizes.size,
    raycastAt: threeHitAt,
  });
  state.seeds = placed.map((p) => p.local);
  rebuildMarkers();
  renderFacts();
  renderStageNote();
  if (!placed.length) toast("no clickable seed positions found in this view — press “Reset view”", "bad");
  return placed;
}

// ─── camera interaction (orbit / zoom) ────────────────────────────────────
function localPx(e) {
  const rect = el.wrap.getBoundingClientRect();
  return [e.clientX - rect.left, e.clientY - rect.top];
}

function beginOrbit(e) {
  const cam = state.cam;
  const d = cam.zAxis;
  state.orbiting = {
    theta: Math.atan2(d[0], d[2]),
    phi: Math.acos(Math.max(-1, Math.min(1, d[1]))),
    x: e.clientX,
    y: e.clientY,
    target: cam.target.slice(),
    viewDist: cam.viewDist,
    zoom: cam.zoom,
    near: cam.near,
    far: cam.far,
  };
}

function orbitTo(e) {
  const o = state.orbiting;
  if (!o) return;
  const dx = e.clientX - o.x;
  const dy = e.clientY - o.y;
  const theta = o.theta - dx * 0.006;
  const phi = Math.max(0.06, Math.min(Math.PI - 0.06, o.phi - dy * 0.006));
  const dir = [Math.sin(phi) * Math.sin(theta), Math.cos(phi), Math.sin(phi) * Math.cos(theta)];
  const pos = C.add3(o.target, C.scale3(dir, o.viewDist));
  state.cam = C.poseCamera({
    position: pos,
    target: o.target,
    zoom: o.zoom,
    width: state.viewport.width,
    height: state.viewport.height,
    near: o.near,
    far: o.far,
  });
  applyCamera();
}

function zoomBy(factor, px, py) {
  if (!state.cam) return;
  const cam = state.cam;
  const zoom = Math.max(0.05, Math.min(600, cam.zoom * factor));
  // keep the world point under the cursor roughly fixed
  const cx = cam.width / 2;
  const cy = cam.height / 2;
  const offsetPx = [px - cx, py - cy];
  const worldOffset = C.add3(
    C.scale3(cam.xAxis, offsetPx[0] / cam.zoom),
    C.scale3(cam.yAxis, -offsetPx[1] / cam.zoom)
  );
  const newPos = C.add3(cam.position, C.scale3(worldOffset, 1 - cam.zoom / zoom));
  const target = C.add3(cam.target, C.scale3(worldOffset, 1 - cam.zoom / zoom));
  state.cam = C.poseCamera({
    position: newPos,
    target,
    zoom,
    width: cam.width,
    height: cam.height,
    near: cam.near,
    far: cam.far,
  });
  applyCamera();
  renderFacts();
  renderStageNote();
}

// ─── events ────────────────────────────────────────────────────────────────
function bindEvents() {
  el.modelSel.addEventListener("change", () => {
    state.modelId = el.modelSel.value;
    loadModel(C.buildModel(state.modelId), { autoSeeds: true, fit: true });
  });

  el.btnPlace.addEventListener("click", () => {
    autoPlace(seedCountValue());
    toast(`placed ${state.seeds.length} seeds (only positions that are cleanly clickable in this view)`, "good");
  });
  el.btnCenter.addEventListener("click", () => {
    if (!state.model) return;
    const hit = threeHitAt(state.viewport.width / 2, state.viewport.height / 2);
    if (!hit) {
      toast("the centre pixel does not hit the mesh in this view", "bad");
      return;
    }
    state.seeds.push(C.faceCentroid(state.model, hit.faceIndex));
    rebuildMarkers();
    renderFacts();
  });
  el.btnClear.addEventListener("click", () => {
    state.seeds = [];
    state.lastEval = null;
    rebuildMarkers();
    renderClickTable(null);
    el.clickMeta.textContent = "— no click yet —";
    renderFacts();
  });
  el.btnFit.addEventListener("click", () => {
    resetView();
    toast("camera re-fitted (CameraFit formula) — note: seeds placed for another view may no longer be clickable", "good");
  });

  el.billboardSel.addEventListener("change", () => {
    state.billboard = el.billboardSel.value;
    renderFacts();
    renderStageNote();
  });
  el.chkMutate.addEventListener("change", () => (state.mutate = el.chkMutate.checked));
  el.chkOverlay.addEventListener("change", () => (state.overlay = el.chkOverlay.checked));
  el.chkInset.addEventListener("change", () => (state.inset = el.chkInset.checked));

  el.modeGroup.addEventListener("click", (e) => {
    const btn = e.target.closest("button[data-mode]");
    if (!btn) return;
    state.mode = btn.dataset.mode;
    for (const b of el.modeGroup.querySelectorAll("button")) b.classList.toggle("on", b === btn);
    applyMarkerColors();
    renderStageNote();
    renderSmear();
  });

  const setMatcher = (value) => {
    state.matcher = value;
    applyMatcher();
    renderClickTable(state.lastEval || null);
    toast(
      value === "s2"
        ? "erase matching = S2 (corrected): 32 px screen tolerance — click a red marker and it erases, even off-surface."
        : "erase matching = S5 (HEAD): ~3 px 3D tolerance and a mesh hit are both required — this is the reported brittleness.",
      value === "s2" ? "good" : "bad"
    );
  };
  el.matcherGroup.addEventListener("click", (e) => {
    const btn = e.target.closest("button[data-matcher]");
    if (!btn) return;
    setMatcher(btn.dataset.matcher);
  });
  el.matcherChoices.addEventListener("change", (e) => {
    if (e.target && e.target.name === "matcher") setMatcher(e.target.value);
  });

  el.btnSelfTest.addEventListener("click", () => runSelfTest(false));
  el.btnCopy.addEventListener("click", async () => {
    const report = state.selfTest ?? window.__forensicReport;
    if (!report) {
      toast("run the self-test first", "bad");
      return;
    }
    const text = report.text ?? JSON.stringify(report, null, 2);
    try {
      await navigator.clipboard.writeText(text);
      toast("report text copied to the clipboard", "good");
    } catch {
      console.log(text);
      toast("clipboard blocked — report printed to the console", "bad");
    }
  });

  // pointer: left = seed action, right/Alt+left = orbit
  el.wrap.addEventListener("pointerdown", (e) => {
    const p = localPx(e);
    if (e.button === 2 || (e.button === 0 && e.altKey)) {
      beginOrbit(e);
      el.wrap.setPointerCapture?.(e.pointerId);
      return;
    }
    if (e.button !== 0) return;
    if (state.mode === "erase") {
      state.erasing = true;
      el.wrap.setPointerCapture?.(e.pointerId);
    }
    handleLeftClick(p[0], p[1]);
  });

  window.addEventListener("pointermove", (e) => {
    if (state.orbiting) {
      orbitTo(e);
      return;
    }
    if (!state.erasing) return;
    const p = localPx(e);
    smearAt(p[0], p[1]);
  });

  window.addEventListener("pointerup", () => {
    state.orbiting = null;
    state.erasing = false;
    renderStageNote();
  });

  el.wrap.addEventListener("wheel", (e) => {
    e.preventDefault();
    const p = localPx(e);
    zoomBy(e.deltaY < 0 ? 1.12 : 1 / 1.12, p[0], p[1]);
  }, { passive: false });

  el.wrap.addEventListener("contextmenu", (e) => e.preventDefault());

  // STL drag & drop (tiny parser)
  const stop = (e) => {
    e.preventDefault();
    e.stopPropagation();
  };
  el.wrap.addEventListener("dragover", (e) => {
    stop(e);
    el.dropHint.classList.add("hot");
  });
  el.wrap.addEventListener("dragleave", () => el.dropHint.classList.remove("hot"));
  el.wrap.addEventListener("drop", async (e) => {
    stop(e);
    el.dropHint.classList.remove("hot");
    const file = e.dataTransfer?.files?.[0];
    if (!file) return;
    try {
      const buf = await file.arrayBuffer();
      const parsed = parseSTL(buf);
      if (!parsed.faces.length) throw new Error("no triangles found");
      const model = C.meshFromArrays(
        "stl",
        `STL · ${file.name}`,
        parsed.vertices,
        parsed.faces,
        "dropped STL (Z-up) — same flat-array layout as meshData"
      );
      const oldId = el.modelSel.value;
      el.modelSel.value = oldId;
      loadModel(model, { autoSeeds: false, fit: true });
      if (model.faceCount > 120000) {
        toast(
          `loaded ${file.name} (${model.faceCount.toLocaleString()} faces). Auto-place/self-test stay on the presets: they need hundreds of raycasts.`,
          "bad"
        );
      } else {
        autoPlace(seedCountValue());
        toast(`loaded ${file.name} — ${model.faceCount.toLocaleString()} faces, ${state.seeds.length} seeds placed`, "good");
      }
    } catch (err) {
      console.error(err);
      toast(`could not parse ${file.name}: ${err.message ?? err}`, "bad");
    }
  });
}

// ─── minimal STL parser (binary + ASCII) ───────────────────────────────────
function parseSTL(buffer) {
  const dv = new DataView(buffer);
  if (buffer.byteLength >= 84) {
    const n = dv.getUint32(80, true);
    if (n > 0 && 84 + n * 50 === buffer.byteLength) {
      const vertices = new Array(n * 9);
      const faces = new Array(n * 3);
      let o = 84;
      for (let i = 0; i < n; i++) {
        o += 12; // per-facet normal
        for (let k = 0; k < 9; k++) {
          vertices[i * 9 + k] = dv.getFloat32(o, true);
          o += 4;
        }
        o += 2; // attribute byte count
        faces[i * 3] = i * 3;
        faces[i * 3 + 1] = i * 3 + 1;
        faces[i * 3 + 2] = i * 3 + 2;
      }
      return { vertices, faces };
    }
  }
  const text = new TextDecoder().decode(buffer);
  const vertices = [];
  const faces = [];
  const re = /vertex\s+(-?[\d.eE+]+)\s+(-?[\d.eE+]+)\s+(-?[\d.eE+]+)/g;
  let m;
  let count = 0;
  while ((m = re.exec(text))) {
    vertices.push(Number(m[1]), Number(m[2]), Number(m[3]));
    faces.push(count++);
  }
  return { vertices, faces };
}

// ─── toast ─────────────────────────────────────────────────────────────────
function toast(msg, kind) {
  el.toast.textContent = msg;
  el.toast.className = `toast show ${kind === "bad" ? "bad" : kind === "good" ? "good" : ""}`;
  clearTimeout(state.toastTimer);
  state.toastTimer = setTimeout(() => (el.toast.className = "toast"), 4200);
}
