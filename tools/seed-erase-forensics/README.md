# Seed-eraser forensic lab

A standalone, runnable exhibit of **ColorYourModel's seed-eraser bug**: the "red sapling"
markers are clickable only inside a ~1.5 px 3D radius and the smear eraser never fires,
because seeds are stored **model-local** while every raycast and projection is in **world**
space.

Everything geometric is transcribed from `src/components/Viewport/Viewport.tsx` at
`feature_012_governance` / `64d544e` — the scene group rotation, `CameraFit`, the
`-π/2`-about-X Z-up→Y-up mapping, the sapling glyph, `seedMarkerSize`,
`eraseNearWorldPoint`'s `2.5 × size` threshold, the billboard bug, and the failed R6/R7/R8
erase strategies.

> This directory only *reads* those formulas. No file under `src/` or `src-tauri/` was
> touched, nothing was committed, and every dependency lives inside
> `tools/seed-erase-forensics/`.

---

## Quick start

```bash
cd tools/seed-erase-forensics
npm i                                     # three@0.170.0 (already installed in this checkout)
python3 -m http.server 8137 --bind 127.0.0.1
# open http://127.0.0.1:8137/index.html
```

The page runs its **auto self-test on load**, so the expected-vs-actual table is already
filled in when it appears.

Headless (no browser):

```bash
node probe.mjs                    # the default model (dome), both aim scenarios, exit 0 = all checks as expected
node probe.mjs --model=box        # box | dome | knot
node probe.mjs --seeds=3 --size=1440x900
node probe.mjs --json             # full report as JSON

node verify-three.mjs             # proves the page's real three.js path ≡ core.mjs
```

Browser smoke test (exercises the two erase-matching modes through the page's real event
path; result in `#smokeOut` and `window.__smoke`, title becomes `SMOKE PASS`):

```
http://127.0.0.1:8137/index.html?smoke=1
```

---

## What the page shows

| Region | Meaning |
| --- | --- |
| Header facts | model, faces, viewport, zoom, `worldPerPx`, `seedMarkerSize`, **erase radius in px**, glyph tilt, seed count, active erase matching |
| Stage | the real scene: dome/box/knot inside `group.rotation = (-π/2, 0, 0)`, orthographic `CameraFit`, wireframe ≤ 20k faces, `AdaptiveGrid`, red saplings (red because erase mode is armed — `seedMaterial` uses `#ff4d4f` in erase mode) |
| Overlay legend | green cross = rendered marker (S2 position) · magenta cross = S1 phantom (local coords read as world) · white cross = the click · green dashed ring = S5 erase radius · cyan dashed ring = S2's 32 px radius · red polyline = the glyph **as drawn** (bug) · cyan dashed = the corrected glyph |
| 8× inset | magnifier on the focused marker: shows the *true* marker size, the glyph orientation bug, and how small the erase radius really is |
| Panel | **This click** — the six-strategy table for the last click, with the value, the strategy's own threshold, the nearest seed and PASS/FAIL; **Erase matching** — which strategy actually drives the eraser; Scene controls; smear counters; click log; engine cross-check |
| Auto self-test | two aim scenarios side by side: expected vs actual per strategy, tolerance in px, hits, evidence |

Interaction: left-click = sow (or erase in erase mode), right-drag / Alt+drag = orbit,
wheel = zoom, `Reset view` re-runs `CameraFit`, drag & drop an `.stl` (binary or ASCII,
Z-up) to load your own mesh.

## The six strategies (locked)

Each row is transcribed from the real code; the point is *which coordinate space each one
compares in*.

| id | Strategy | Code path | Space used | Threshold | Expected |
| --- | --- | --- | --- | --- | --- |
| S1 | `project(local seed)` → screen px | `Viewport.tsx` R6/R7 ~1939–1968 | **model-local seed** fed to a world-space projection | 32 px | FAIL |
| S2 | `project(localToWorld(seed))` → screen px | corrected R6/R7 | world ↔ world | 32 px | PASS |
| S3 | `ray.distanceToPoint(local seed)` | R8-first ~1950–1963 | world ray vs **local seed** | 32 px | FAIL |
| S4 | `ray.distanceToPoint(world seed)` | corrected R8-first | world ↔ world | 32 px | PASS |
| S5 | `worldToLocal(hit).distanceTo(local seed)` | R8-FINAL click path :1991 → `eraseNearWorldPoint` :1076 | local ↔ local | `2.5 × seedMarkerSize` (≈ **1–4 px**) | PASS (exact aim only) |
| S6 | `hit(world).distanceTo(local seed)` | R8-FINAL smear path :2125 | **world** hit vs **local** seed | same `2.5 × size` | FAIL |

`world = (x, z, −y)` and `local = (X, −Z, Y)` are the scene group's rotation.

## Erase matching (the exhibit that proves the fix)

The stage bar has a two-way switch that drives what the lab's eraser *actually does*:

* **`S5 · HEAD` (default)** — app-faithful: the click must land on the mesh **and** within
  `seedMarkerSize × 2.5` world units of the model-local centroid (a ~1.5 px 3D tolerance at
  fit zoom). Off-centre clicks and clicks on the glyph (no mesh hit → `getLocalHit()`
  returns `null`) do nothing. This is the reported brittleness.
* **`S2 · fixed`** — the corrected match: `project(localToWorld(seed))` vs the pointer,
  32 px glyph radius. It erases reliably, even when the pointer is on the glyph and the ray
  misses the surface.

The smear path follows the same switch: `S6` (world hit vs local seed — never matches) at
HEAD, or the corrected screen-space `S4` test (ray→world seed, 32 px) in S2 mode.

## Auto self-test

Seeds are placed deterministically (screen grid → face centroid, kept only if the rounded
pixel re-hits within the S5 tolerance), then clicked at:

1. **centred** — the integer pixel of the marker's rendered position (S2 projection);
2. **4 px off** — the same pixel +4 px in x, a realistic human aim.

Expected, per scenario:

| Scenario | S1 | S2 | S3 | S4 | S5 | S6 |
| --- | --- | --- | --- | --- | --- | --- |
| centred | FAIL | PASS | FAIL | PASS | **PASS** | FAIL |
| 4 px off | FAIL | PASS | FAIL | PASS | **FAIL** | FAIL |

S5 is expected to *stop working* once the aim error exceeds its tolerance — that is the
whole finding, and the table prints S5's tolerance in screen px (`0.2400 wu ≈ 3.25 px` on
the dome at 1280×800) next to S2/S4's fixed 32 px. The self-test writes the full report to
`window.__forensicReport` (also `Copy report JSON`); `core.mjs`'s `formatReport()` prints
the same table to stdout via `node probe.mjs`.

`node verify-three.mjs` additionally proves, against the real three.js r170 build, that the
page's camera/raycaster/`worldToLocal`/billboard measurements agree with `core.mjs`
(≤ 1e-5 wu; three intersects the app's Float32 position buffer while `core.mjs` uses the
float64 source), and prints the numeric "erase matching" sweep:

```
 offset   S5 (HEAD) — seed found         S2 (corrected) — seed found
 +0px     all 5 markers                  all 5 markers
 +1px     all 5 markers                  all 5 markers
 +2px     4/5 (misses 1)                 all 5 markers
 +3px     1/5 (misses 4)                 all 5 markers
 +4px     0/5 (misses 5)                 all 5 markers
 ...
 +31px    0/5 (misses 5)                 all 5 markers
 +35px    0/5 (misses 5)                 0/5
```

## Files

| File | Role |
| --- | --- |
| `core.mjs` | pure math, no three.js/DOM: `CameraFit`, Z-up mapping, projection, ortho ray, Möller–Trumbore raycast, glyph, strategies, deterministic placement, two-scenario report. **Source of truth for every number.** |
| `app.mjs` | three.js scene + real `Raycaster`/`worldToLocal` path, overlay & 8× inset, HUD, matcher, self-test, STL drop |
| `index.html`, `style.css` | the page |
| `probe.mjs` | headless table exactly as the page computes it (`core.mjs` only) |
| `verify-three.mjs` | headless three.js r170 cross-check + matcher sweep + smoke of the `?smoke=1` claims |
| `package.json` | three@0.170.0, `npm run probe` / `npm run verify` / `npm run serve` |

## Fidelity notes / deliberate deviations

* Faithful: `CameraFit` lines 174–217 verbatim; R3F's default symmetric orthographic
  frustum (`±size/2`) with `zoom = min(min(w,h)/effectiveDim*0.88, 500)`;
  `worldPerPx = (top−bottom)/zoom/height = 1/zoom`; `seedMarkerSize = max(0.0054, diag·0.0016)`;
  erase threshold `size × 2.5`; `getLocalHit` = raycast + `worldToLocal`; `SeedMarker`
  geometry/scale/positions; billboard = `inner.quaternion.copy(camera.quaternion)` inside the
  rotated group (measured: **90.000° off camera**); face-centroid seed storage from the flat
  `vertices`/`faces` arrays; `DoubleSide`; `SceneSetup` lights; `AdaptiveGrid` sizing/placement;
  wireframe at 0.05 opacity for small meshes.
* Cosmetic / not part of the exhibit: the model presets are synthetic (a dome ≈ 1 920 tris,
  a 12-tri box, a 48 000-tri torus knot generated point-for-point from
  `TorusKnotGeometry`), the comment says the wireframe is skipped above 20k faces, and
  `?smoke=1` is test scaffolding.
* Views: after resizing or orbiting, seeds placed for the previous view may no longer be
  clickable (that is S5's tolerance, faithfully) — press `Reset view` and `Auto-place` again,
  or just run the self-test, which re-fits the view first.
* No WebGL → nothing renders (the page needs a WebGL2 context); `probe.mjs` /
  `verify-three.mjs` need no GPU.

## Self-check (what "green" means)

```bash
node probe.mjs       # exit 0, "SELF-TEST: PASS — ... 12/12 checks"
node verify-three.mjs# exit 0, "ALL 12 CHECKS PASSED"
curl -s http://127.0.0.1:8137/index.html >/dev/null   # page is served
```
