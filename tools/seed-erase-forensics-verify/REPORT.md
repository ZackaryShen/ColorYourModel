# Seed-erase coordinate-space forensics — INDEPENDENT verification

**Author:** verifier (independent teammate). **Scope:** `tools/seed-erase-forensics-verify/` only.
No `src/` or `src-tauri/` file was modified. No git commit.

**Independence:** `probe.mjs` was written and produced numbers *before* `tools/seed-erase-forensics/`
(the app builder's deliverable) was opened — it was in fact never opened during this work. Every
number below comes from `probe.mjs`, real `three@0.170.0`, and the app source read directly:
`src/components/Viewport/Viewport.tsx` at HEAD `64d544e`, `src/hooks/useMesh.ts`,
`src-tauri/src/mesh/loader.rs`, `src/store/appStore.ts`, `src/components/SeedPanel.tsx`.

---

## 1. What is being tested

The working hypothesis under review: *the seed eraser always misses because camera-space math is fed
model-local (STL Z-up) seed coordinates as if they were world coordinates*. The scene has a
`<group rotation={[-PI/2,0,0]}>` between the raw STL geometry and the world, so
`world = R * local`, `R = R_x(-90°)`, and for `p = (x,y,z)`:

```
R p        = (x,  z, -y)
R p - p    = (0, z-y, -y-z)
|R p - p|  = sqrt(2 * (y^2 + z^2))        (independent of x; zero only on the X axis)
```

Six strategies were reconstructed from the git history of `Viewport.tsx` (they are all in the
handoff §3.4 or in `git show` of commits `73ec499`, `b98e869`, `df094b7`):

| id | strategy | status in history |
|----|----------|-------------------|
| S1 | `project(seedLocal)` → canvas px, 32 px | R6/R7 (wrong space) |
| S2 | `project(localToWorld(seed))` → canvas px, 32 px | **never shipped** — the corrected projection |
| S3 | `raycaster.ray.distanceToPoint(seedLocal)/worldPerPx`, 32 px | R8-first (`b98e869`, wrong space) |
| S4 | `raycaster.ray.distanceToPoint(localToWorld(seed))/worldPerPx`, 32 px | **never shipped** — corrected ray |
| S5 | `\|worldHit - localToWorld(seed)\| <= 2.5*size` | HEAD click path (local hit + local seed) — coordinate-correct |
| S6 | `\|worldHit - seedLocal\| <= 2.5*size` | HEAD smear path, `Viewport.tsx:2126` — **world hit vs local seed** |

The app's constants: `size = max(0.0054, diag*0.0016)` (`Viewport.tsx:1059-1066`),
erase threshold `size*2.5` (`Viewport.tsx:1090`), screen strategies `<= 32 px`.

## 2. Assumptions (each sourced / justified)

1. **Geometry is raw STL Z-up.** `src-tauri/src/mesh/loader.rs:load_stl` copies `stl_io` vertices
   unchanged (`merged_vertices.push([vert[0],vert[1],vert[2]])`) into `MeshModel.vertices`;
   `useMesh.ts:29-57` writes those into the `BufferGeometry` position attribute unchanged. No axis
   transform anywhere.
2. **One rotation, no translation, between geometry and world.** `Viewport.tsx:2580`
   `<group rotation={[-Math.PI/2, 0, 0]}>` contains the `<mesh>` (`:2581`) and the seed markers
   (`:2674`). Mesh position is default `(0,0,0)`; `MeshDisplay`'s group is a direct scene child.
   `getLocalHit` does `meshRef.current.worldToLocal(h.point)` (`:1457`), i.e. it returns STL-local
   coordinates — the same space the seeds are stored in.
3. **Seeds are stored model-local.** Manual seeds are written from `meshData.vertices` face
   centroid (`Viewport.tsx:2065-2078`); the face arrays are the same raw vertices the geometry is
   built from.
4. **Ortho frustum is CSS-pixel sized.** R3F v8.17.10, `packages/fiber/src/core/utils.ts`
   (`updateCamera`):
   ```ts
   if (isOrthographicCamera(camera)) {
     camera.left = size.width / -2
     camera.right = size.width / 2
     camera.top = size.height / 2
     camera.bottom = size.height / -2
   }
   ```
   with `size` = CSS pixels measured from the canvas' parent (`core/index.tsx`, `computeInitialSize`).
   The camera is created as `new THREE.OrthographicCamera(0,0,0,0,0.1,1000)` and the app passes only
   `{position, zoom}` (no `left/right/top/bottom`, so `camera.manual` stays false and R3F keeps
   re-deriving the frustum on every resize). Therefore:
   `world-per-px = (top-bottom)/zoom/height = 1/zoom`, and the app's own
   `fit = min(w,h)/effectiveDim; zoom = fit*0.88` (`Viewport.tsx:205-206`) is exactly "fit
   effectiveDim into the short viewport edge". Probe check `scene.ortho-frustum`: 0.240486 vs
   0.240486 (delta < 1e-12).
5. **Camera pose = initial CameraFit pose** (`dir=(0.4,0.4,1).normalize()`,
   `viewDist=maxDim*10`, `lookAt(center)`, `near=max(0.1,viewDist-maxDim*6)`,
   `far=viewDist+maxDim*6`). Orbiting changes the error vector but not its order of magnitude.
6. **Canvas 1400×860 CSS px** (Tauri default window 1400×900 minus chrome). The observed px bands
   scale linearly with `zoom`, i.e. with `1/canvasShortEdge`, so another canvas height shifts the
   band; it does not change any conclusion.
7. **Committed seed material `depthTest: true`** (`Viewport.tsx:2717`), `depthFunc` = three's
   default `LessEqualDepth` — coplanar glyph fragments still pass, so the glyph is "visible" unless
   it is behind the surface.
8. Two synthetic non-indexed STL-Z-up meshes (box + sphere, 4748 faces each), built in the probe:
   - **Model A** (the spec'd mesh): 40×60×30 box + r=11 sphere on top.
   - **Model B** (handoff §3.1 scale): 140×120×100 box + r=30 sphere on top. Its
     `seedMarkerSize = 0.361 wu` (handoff says ≈0.33) and fitting at 1400×860 gives zoom 4.158,
     0.2405 wu/px.

## 3. A — wrong projection is hundreds of px and non-linear

`project(stored local seed)` vs `project(R * stored seed)` (what the marker actually renders at),
over ~2400 surface points:

| model | min | p25 | median | p75 | p95 | max | share inside observed 235–652 px |
|---|---|---|---|---|---|---|---|
| A 40×60×30 box+sphere (fit zoom 9.703) | 70 px | 285 | **385** | 485 | 525 | **532** | 99.1 % |
| B 140×120×100 box+sphere (fit zoom 4.158) | 60 px | 432 | **549** | 666 | 713 | **721** | 71.4 % |

* The handoff's observed band `235..652 px` (R7) is exactly this quantity for model B at fit zoom:
  71.4 % of the surface lands inside it, median 549 px, max 721 px.
* **It is not linear in click position:** under ortho projection, a fixed world offset projects to a
  fixed px offset. `(0,20,60)`, `(70,20,60)`, `(-70,20,60)` — three screen positions 270 px apart —
  all give **290.987 px** error. The dependence is on `(y,z)` of the seed, not on where the seed is
  on screen, which is exactly the R7 observation "does not vary linearly with click position".
* It **scales linearly with zoom** (290.99 px @ zoom 4.158 → 581.97 px @ 8.316, ratio 2.000000) and
  is **pan-invariant** under ortho (same value after panning camera+target 120 wu).

## 4. B — simulate the real click on the rendered marker (S1..S6)

Click at the projected centre of the rendered marker (`project(R p)` → NDC → px → raycast the mesh →
`worldHit`, `localHit = mesh.worldToLocal(worldHit)`), i.e. exactly where the user clicks the red ball:

| seed (STL local) | incidence | S1 wrong-proj px | S2 corrected-proj px | S3 wrong-ray px | S4 corrected-ray px | S5 world-hit vs world-seed wu | S6 world-hit vs local-seed wu |
|---|---|---|---|---|---|---|---|
| (23.3,-60.0,33.3) | 29.5° | 330.6 **FAIL** | 0.000 **PASS** | 330.6 **FAIL** | 0.0000 **PASS** | 0.0000 **PASS** | 97.07 **FAIL** |
| (70.0,20.0,33.3) | 69.6° | 148.6 **FAIL** | 0.000 **PASS** | 148.6 **FAIL** | 0.0000 **PASS** | 0.0000 **PASS** | 54.97 **FAIL** |
| (9.5,-28.1,104.1) | 13.4° | 628.6 **FAIL** | 0.000 **PASS** | 628.6 **FAIL** | 0.0000 **PASS** | 0.0000 **PASS** | 152.51 **FAIL** |
| (-18.2,-23.7,101.7) | 59.6° | 605.5 **FAIL** | 0.000 **PASS** | 605.5 **FAIL** | 0.0000 **PASS** | 0.0000 **PASS** | 147.62 **FAIL** |
| (-2.8,5.0,129.4) | 81.3° | 697.3 **FAIL** | 0.000 **PASS** | 697.3 **FAIL** | 0.0000 **PASS** | 0.0000 **PASS** | 183.16 **FAIL** |

The predicted pattern — **S1/S3/S6 fail, S2/S4/S5 pass** — holds exactly.
Also measured, algebraically: under an ortho camera **S1 ≡ S3** (max |S1−S3| = 0.0e0) and
**S4 ≡ S2** (0.0e0). The "picking ray" strategy `b98e869` was therefore not an independent
strategy at all: it computed the same pixel number as the broken projection.

With a realistic ±2 px hand jitter the corrected strategies stay clean (S2/S4 = 2.00 px, PASS) while
**S5 already misses on non-face-on seeds**: 0.941 wu vs the 0.9024 wu threshold at 59.6° incidence,
1.37 wu at 69.6°, 1.13 wu at 69.6° diagonal (threshold = 2.5*size*cos(incidence) shrinks faster than
a 2 px hand jitter). That is the first crack in "the current path is correct".

## 5. C — smear path failure magnitude (`Viewport.tsx:2126`)

`onPointerMove` calls `eraseNearWorldPoint(hitSmear.point)` where `hitSmear` is a **world-space**
raycast point, while `eraseNearWorldPoint` compares against seeds stored in the **model-local** STL
frame. The distance is therefore:

```
d = |worldHit - seedLocal| ~= |R p - p| = sqrt(2*(y^2+z^2))
```

| seed local | predicted \|R p − p\| wu | measured S6 dWorld wu | over 2.5*size threshold | px @fit zoom |
|---|---|---|---|---|
| (23.3,-60.0,33.3) | 97.07 | 97.07 | **107.6×** | 404 px |
| (70.0,20.0,33.3) | 54.97 | 54.97 | **60.9×** | 229 px |
| (9.5,-28.1,104.1) | 152.51 | 152.51 | **169.0×** | 634 px |
| (-18.2,-23.7,101.7) | 147.62 | 147.62 | **163.6×** | 614 px |
| (-2.8,5.0,129.4) | 183.16 | 183.16 | **203.0×** | 762 px |

The closed form is confirmed to 0.0e0 residual. The error is model-scale (up to 183 wu ≈ model size;
matches the handoff's "~133 wu ≈ whole model"), it is **seed-dependent** (zero only for seeds on the
world X axis, i.e. STL `y=z=0`) and it never fires while dragging. This is the actual code defect at
HEAD.

## 6. C-ray — the "ray data is garbage" claim is REFUTED

The R8-first diagnosis ("picking-ray distance is garbage, 549 → 13293 px, grows with pan") is fully
explained by the same wrong coordinate space. For a fixed seed the wrong-ray dPx is exactly
`|(I - d dᵀ)(R-I)p| / wuPerPx = offset * zoom`, so it grows linearly with zoom (the hidden variable),
and pan adds the fixed wrong point's displacement. Measured on a box-front seed with screen-plane
mismatch **79.49 wu**, with the marker panned to the view centre (what a user does before zooming):

| handoff observation | zoom solved by probe | wu/px | S3 wrong-ray px | S4 corrected-ray px | deviation |
|---|---|---|---|---|---|
| **549 px** | 6.91 | 0.14480 | **549** | 0.0000 | 0.00 % |
| **13293 px** | 167.22 | 0.00598 | **13293** | 0.0000 | 0.00 % |

Both observed endpoints reproduce to 0.00 % at legal app zooms (fit 4.16 … clamp 500). The corrected
ray (S4) is **0.0000 px at every zoom**, because the ray and the world marker move together. The
picking ray was never broken; only the point it was compared to was in the wrong space.

## 7. D — aim tolerance in px (including grazing incidence)

`r_px = size * zoom` = 1.50 px at fit zoom, 14.44 px at zoom 40. Screen strategies S2/S4 have a flat
32 px tolerance at any incidence. The world-distance strategy S5 (the current click path) has:

```
tol_px = 2.5 * r_px * cos(incidence)          (validated on a real tilted plane, ≤4.7 % error)
```

| incidence (normal vs view) | cos | tol px @fit | tol px @zoom40 | tol / marker radius | sphere radius clickable | consequence |
|---|---|---|---|---|---|---|
| 0° | 1.000 | 3.75 | 36.10 | 2.50 r | 100 % | whole visible sphere |
| 28.6° | 0.878 | 3.29 | 31.69 | 2.20 r | 100 % | whole visible sphere |
| 53.1° | 0.600 | 2.25 | 21.66 | 1.50 r | 100 % | whole visible sphere |
| 66.4° | 0.400 | 1.50 | 14.44 | 1.00 r | 100 % | edge exactly at limit |
| 78.5° | 0.200 | 0.75 | 7.22 | **0.50 r** | 50 % | sphere edge not clickable |
| 84.3° | 0.100 | 0.38 | 3.61 | **0.25 r** | 25 % | sphere edge not clickable |

Numeric validation (actual raycast against a plane tilted to the given incidence, zoom 40): max
click offset 36.0 / 31.0 / 18.0 / 9.0 / 3.0 px vs analytic 36.1 / 31.3 / 18.0 / 9.3 / 3.1 px
(0.27 %…4.6 %). S6 (smear) has tolerance **0 px**: it never fires.

So: the corrected projection strategy (S2) gives a **~10× larger, angle-independent aim tolerance**
than the current world-distance click path (S5), and S5 degrades precisely on grazing surfaces.

## 8. E — the glyph the user aims at

The marker is a 3-D "sapling" (seed sphere r=size, stem to y=3*size, leaves at ±0.55*size) drawn in a
child group that is billboarded by copying the **camera world quaternion** into the child's **local**
quaternion (`Viewport.tsx:2789`). Its parent chain includes the −90°X root group, so the correct
billboard quaternion is `parentWorldQuat⁻¹ * cameraWorldQuat`; the current code is off by the parent
rotation:

| metric | correct billboard | current code (bug) |
|---|---|---|
| glyph plane normal vs camera direction | 0.00° | **83.04°** |
| world orientation error | — | **83.04°** |
| stem direction on screen (0° = up) | 0.00° | **85.81°** |
| stem screen foreshortening | 1.0000 | **0.2287** |
| glyph screen height @fit zoom | 6.57 px | **2.99 px** |
| seed → glyph top @fit zoom | 5.07 px | **1.49 px** |
| depth-test visible fraction / buried | 27.9 % / 72.1 % | 28.6 % / 71.4 % |
| visible-glyph centroid offset from seed | 0.36 px | 0.37 px |

Per-part, face-on seed: **sphere 48.8 % visible; stem 0 %; leaf 0 %** — the whole sapling is buried
inside the surface, the user's "red dot" is effectively the front hemisphere of the seed sphere. The
bug does **not** move the visible target centre away from the seed (centroid 0.37 px), so it is not
the cause of the miss; it makes the glyph read as a deformed red blob and removes the sprout cue
(2.99 px vs 6.57 px tall at fit zoom; 28.8 px vs 63.2 px at zoom 40).

Aim point vs accept region: the sapling's top sits **3.385 marker radii** above the seed, while S5
accepts only `2.5*cos` radii face-on (2.18 r for the measured seed) — **35.7 % of the glyph's height
is outside the clickable world-distance radius** and 100 % of stem+leaf area is here. On a grazing
seed (81°), S5 accepts only **38 % of the visible sphere radius**, while the correct billboard glyph
in that pose visibly floats off the surface (median gap +2.0 wu) — so a user aiming at the floating
glyph can be well outside S5's tolerance.

## 9. Verdict

**CONFIRMED (and the sole cause of the R6/R7/R8-first failures).** Every historical miss number is
reproduced by treating the stored STL-local seed as a world point: 549 px and 13293 px ray distances
reproduce to 0.00 % at zoom 6.91 / 167.22, the 235–652 px projection band is the model-B fit-zoom
error distribution, and correcting only `localToWorld()` makes the same comparisons pass at
<0.001 px with a flat 32 px aim tolerance.

**PARTIALLY CONFIRMED as the explanation of the current "never fires" report**, because the HEAD
click path is already coordinate-correct (S5) and a plain click on the visible red half-ball passes.
Things the hypothesis does not cover, which the app must explain/fix:

1. **Smear path bug (the remaining real defect):** `Viewport.tsx:2126` passes the *world* hit into
   the local-seed matcher — 61×–203× over threshold, always misses while dragging.
2. **Auto-mode silently disables erasing:** the `segmentMode === "auto"` early return
   (`Viewport.tsx:1874`) runs *before* the erase branch; switching `segmentMode` does not clear
   `seedEraseMode` (`appStore.ts:732-742`, unlike `setActiveTool` at `:637-648`) and the eraser
   cursor is shown regardless of mode (`Viewport.tsx:1142`). Result: eraser cursor, click does
   nothing, `seed.autoNoSowing` status.
3. **Successful click looks like a miss:** `pointerdown` sets `eraseDragRef = true`
   (`Viewport.tsx:1993`); the next `pointermove` (even 1 px) enters the smear branch, which logs
   `erase miss: nearest seed <huge> wu` and overwrites the "erased" status message. The miss
   diagnostic for the WORLD-vs-LOCAL bug therefore fires on every drag, while the first click may
   actually have erased.
4. **Even a corrected click path with S5 tolerance is not enough:** tolerance = `2.5*r_px*cos` — on
   a 66°+ surface the visible sphere edge is unclickable, at 84° only 25 % of the radius is. A
   corrected S2/S4 screen match (projection with `localToWorld`) with the existing 32 px threshold
   is angle-independent and 10× more forgiving.
5. **The billboard quaternion is wrong** (83° orientation error, 0.23× foreshortening, sapling
   effectively invisible); fix `inner.quaternion.copy(parentWorldQuat⁻¹ * cameraWorldQuat)` and
   consider a flat sprite with `depthTest:false` for the glyph — the current 3-D sprout is 72 %
   buried, so only the sphere hemisphere is a reliable target.

## 10. How to reproduce

```bash
cd tools/seed-erase-forensics-verify
npm init -y && npm i three@0.170.0   # only dependency, installed in this directory
node probe.mjs; echo $?             # prints all tables; exit 0 only if all 40 checks pass
```

`probe.mjs` asserts 42 expectations (scene assumptions, A.1–A.7, B.1–B.9, C.1–C.6, D.1–D.6,
E.1–E.12). It exits 1 if any fails: run it after any change to the reconstruction to confirm the
numbers still hold. Canvas/zoom/pan are parameters; changing them moves the px bands but not the
fail/pass pattern or the closed forms.

---

## 11. Small corrections / cross-checks against the archaeology audit

Written after `probe.mjs` was frozen (independence preserved); the auditor's doc
`docs/superpowers/analysis/2026-11-07-seed-erase-coordinate-space.md` agrees with the verdict, with
two mechanism details worth correcting:

1. **R6's screen threshold was 26 px, not 32 px** (`git show 20088d3`: `bestD <= 26`); the R7/R8-era
   code uses 32 px (`git show 73ec499` still 26, `b98e869` 32). This probe uses 32 px per the task
   spec / HEAD-era dead code. The choice is immaterial: the wrong-space distances are 149–721 px,
   i.e. 5–28× the threshold.
2. **R6 also had a frame bug** (window-absolute `clientX/Y` vs canvas-relative px, fixed in
   `73ec499`) worth a constant `hypot(rect.left, rect.top)` of order tens of px on the observed
   layout. It is not modelled here because it is a constant screen offset and cannot produce a
   seed-dependent 235–652 px band; the dominant term is the wrong coordinate space.
3. **The auditor's R8-first mechanism (§4.3) is inaccurate.** `Ray.distanceToPoint` is the
   *perpendicular* distance to the infinite ray — three r170 `Ray.distanceSqToPoint` clamps only
   `t<0` — so the metric is not anchored at the camera and is not bounded by `10*maxDim`. For an
   ortho camera (`Raycaster.setFromCamera` puts the origin in the camera plane), the perpendicular
   distance is the view-plane offset between the click ray and the local seed read as world, and it
   is exactly `|(I-ddᵀ)(R-I)p| / wuPerPx = offset * zoom`. Our raycast reproduction pins the two
   observed endpoints at zoom 6.91 (549 px) and zoom 167.22 (13293 px) with 0.00 % deviation
   (section 6), which is a stronger statement than "the value is set by 10*maxDim and zoom".
4. **Additional UX trap the audit does not list:** because `pointerdown` arms `eraseDragRef`
   (`Viewport.tsx:1993`), *any* ≥1 px pointer movement before release enters the smear branch and
   overwrites the "erased" status with `erase miss: nearest seed <huge> wu` (from the world-hit vs
   local-seed comparison). So on HEAD a *successful* click can report a miss — the user-facing
   evidence "it never fires" is not proof that the click branch failed.

## 12. Confidence & limits

- High confidence in the closed forms (`|R p − p| = √2·ρ_yz`, `tol_px = 2.5·r_px·cos`, `S1 ≡ S3`,
  `S4 ≡ S2` under ortho) — they are checked numerically to 1e-13 in the probe.
- Medium confidence in absolute px values: they depend on the real canvas CSS size, the real model
  bbox/diagonal, and the camera zoom at the moment of the click, none of which were captured from a
  failing session. The probe shows the *laws* (band ≈ zoom-scaled rotation displacement; endpoints
  reproduced at legal zooms); the exact observed numbers will differ if the window is smaller or the
  model is larger.
- The probe does not model: camera orbit during seeding (changes the direction `d` and hence the
  offsets), multi-part models whose bbox is far from the local origin (would increase the smear
  error), or R3F dpr (irrelevant: raycasting and projection both use CSS px).
