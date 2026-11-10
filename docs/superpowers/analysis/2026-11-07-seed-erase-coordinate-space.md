# Seed-erase coordinate-space audit

> Date: 2026-11-07 · Branch `feature_012_governance` · HEAD `64d544e` (code under test: `df094b7`)
> Author: teammate "archaeologist" (read-only audit of `src/` and `src-tauri/`)
> Scope: why the seed eraser has missed across R6/R7/R8-first, and what coordinate space every
> seed producer/consumer actually uses. **No source file was modified.** All `Viewport.tsx` line
> numbers refer to HEAD `64d544e` unless an older commit is named explicitly.
>
> **Status (2026-11-10, round 9):** the corrections in §8 are now implemented —
> `src/components/Viewport/Viewport.tsx` (screen-space matcher, smear fix, auto-mode ordering,
> billboard) plus `src/utils/seedErase.ts` (+14 unit tests). See the CHANGELOG entry. Sections 0–7
> below remain the *pre-fix* evidence and were deliberately not rewritten.

---

## 0. Verdict (TL;DR)

1. **The Lead's hypothesis is CONFIRMED, with one refinement.** Stored seed coordinates are
   **model-local (STL raw vertex space, Z-up)**. `Vector3.project()` and `Raycaster` are
   **world-space**, and the only model transform is the group rotation `Rx(-π/2)` at
   `src/components/Viewport/Viewport.tsx:2580`. Treating a local seed as a world point displaces
   it by exactly `√2·ρ_yz` (its distance from the local X axis) — for the measured model that is
   **119 wu worst-case, same order as the whole model**, which reproduces the R7 `235–652 px` and
   R8-first `~133 wu` numbers (numeric reconstruction in §4).
   - *Refinement:* R6 had a **second, independent bug on top** — it passed window-absolute
     `e.clientX/Y` into a canvas-relative comparison (`20088d3`), worth ~59 px. That offsets the
     handoff's claim that R6 proved "`project()` is systematically wrong by 244 px"; the projection
     call was never the problem, the input space was.
2. **The "erase branch never executes" hypothesis is REFUTED.** The R7 measurement itself
   (`nearestSeedScreen` vs click = 235–652 px) can only be produced *inside* the erase branch, so
   the branch ran. R6's "no HUD log" is fully explained by R6 having **no `log.info` statement at
   all** in that branch (`git show 20088d3`) — only status-bar messages. See §7.
3. **Live bug inventory at HEAD:** exactly **one** live space-mixing site — the **smear path**
   `Viewport.tsx:2126` (world hit → local seed match). The click path `Viewport.tsx:1992` is
   space-consistent (local hit vs local seed) and *should* erase. In addition there is
   **dead code** at `1931–1990` (`eraseAtScreen`, R8-first's world-ray math, now never called) that
   encodes the refuted assumption, and the **billboard bug** at `2789` (orientation only).
4. **A separate, surviving geometric weakness** (not a coordinate-space bug): at HEAD the click
   tolerance is `2.5 × seedMarkerSize ≈ 0.825 wu`, which is only **≈3 px** at the auto-fit zoom;
   and because of the billboard bug the visible marker is only the sphere (≈2.6 px dot), not the
   sapling (§6). The click path is correct in *space* but tight in *tolerance*.

---

## 1. Evidence base

| Evidence | Location |
|---|---|
| Handoff, round-8 history & numbers | `docs/superpowers/handoff/2026-11-02-governance-round8-handoff.md:92-135` (§3.1–§3.4) |
| Group rotation (only model transform) | `src/components/Viewport/Viewport.tsx:2579-2580` |
| Mesh is a child of that group | `Viewport.tsx:2581` |
| Markers render inside that group | `Viewport.tsx:2672-2676`, `2750-2758` |
| Local↔world conversion comment + function | `Viewport.tsx:1423-1463`, `1457` |
| Local↔world mapping used for camera fit | `Viewport.tsx:175-183` |
| Commits R6..R8-final | `20088d3` (R6), `73ec499` (R7 offset), `7b080e9` (R7 probe), `b98e869` (R8-first), `df094b7` (R8-final) |

Numbers used repeatedly below, each derived in-place:

- `seedMarkerSize = max(0.0054, 0.0016 × bbox diagonal)` — `Viewport.tsx:1059-1066`, specifically
  `1065`. Handoff §3.1 measures ≈0.33 wu on the 148k-face model ⇒ **bbox diagonal D ≈ 0.33/0.0016
  = 206 wu**.
- erase threshold `= seedMarkerSize × 2.5` — `Viewport.tsx:1090` ⇒ **0.825 wu**.
- `zoom = min(viewportW, viewportH) / effectiveDim × 0.88`, capped at 500 — `Viewport.tsx:203-206`.
- Canvas is `orthographic` with no manual frustum — `Viewport.tsx:3441-3443` — so the frustum is
  R3F's default `±size/2`, i.e. **1 world unit = `zoom` px**, `worldPerPx = 1/zoom` (this is also
  exactly the formula the code itself uses at `1950-1953`).

---

## 2. The one transform that matters (exact algebra)

`Viewport.tsx:2579-2580`:

```tsx
<group rotation={[-Math.PI / 2, 0, 0]}>
```

`MeshDisplay` is a direct child of `<Canvas>` (`Viewport.tsx:3457`), its JSX root is that group,
and the group has **no position and no scale**. Therefore, for any point inside it:

```
world = R · local,   R = Rx(-π/2) = [[1,0,0],[0,0,1],[0,-1,0]]
local = (x, y, z)  →  world = (x,  z, -y)          # comment Viewport.tsx:175-176
world = (X, Y, Z)  →  local = (X, -Z,  Y)          # comment Viewport.tsx:1423
```

Computed (node script, `/tmp/seed-space-math.mjs`):

```
local p  = (12, 40, 70)
world    = (12, 70, -40)          # matches Viewport.tsx:175-176
back     = (12, 40, 70)           # matches Viewport.tsx:1423, round-trip error 0
```

**Wrong-space penalty.** If a local point is consumed as if it were world (or vice versa), the
error is `|R·p − p|`:

```
R·p − p = (0, z−y, −y−z)
|R·p − p| = √((z−y)² + (y+z)²) = √2 · √(y²+z²) = √2 · ρ_yz
```

where `ρ_yz` is the point's distance from the **local X axis**. Two consequences:

- The error is **not** a screen-space distortion — it is a fixed 3-D displacement per seed, of the
  order of the model itself. For a cube of body diagonal D the worst-case is exactly **D/√3**
  (computed: D=206.25 ⇒ **119.1 wu**; a flatter/offset model can exceed it, since `ρ_yz` includes
  any model offset from the local origin).
- It is **not linear in click position** — it is fixed per seed. That is precisely the R7
  observation "difference 235–652 px and it does not vary linearly with click position"
  (handoff `:128`).

---

## 3. Coordinate-space inventory

### 3.1 Producers of seed coordinates — all MODEL-LOCAL

| Producer | Space | Evidence |
|---|---|---|
| Manual click seed (`addSeedPoint`) | **local** | Centroid of the clicked face from `meshData.vertices` (flat STL vertex array): `Viewport.tsx:2063-2078` (`2065-2077`). `meshData.vertices` is the raw STL vertex list; the Three geometry is built from it verbatim: `src/hooks/useMesh.ts:38-41`. R5 already established "seed = face centroid" (handoff `:85-86`). |
| `recommend_seeds` suggestion | **local** | Rust: `SeedSuggestion.point` documented "a model-local point": `src-tauri/src/segment/recommend.rs:41-49`; built as `point: centroids[i]` at `recommend.rs:279-285`. IPC maps `point[0..2] → x,y,z`: `src/hooks/useTauriCommand.ts:550-556`. |
| `detect_planar_regions` seed | **local** | `seed: SeedSuggestion { point: seed_point, .. }` — `src-tauri/src/segment/planar.rs:384`; mapped at `useTauriCommand.ts:599`. |
| `detect_multiview_regions` seed | **local** | `src-tauri/src/segment/multiview.rs:411`; mapped at `useTauriCommand.ts:641`. |
| `eyeRegions` → `eyeSeeds` (grow input only) | **local** | Computed in the frontend as the centroid of a region face from `meshData.faces`/`vertices`: `src/components/SeedPanel.tsx:781-812`; only concatenated for `seed_grow` at `SeedPanel.tsx:829`. It is **never** inserted into `seedPoints`/`suggestedSeeds`, so the eraser cannot target it (see §9). |
| Accepted suggestion (promotion to `seedPoints`) | **in place, local** | `acceptSuggestedSeed` copies the picked suggestion object unchanged: `src/store/appStore.ts:751-767`. |

The type system documents the same thing: `SeedPoint` doc says "the snapped 3D position
(model-local)" — `src/types/mesh.ts:40-49`; `PlanarRegion.seed` and `MultiViewRegion.seed` are
`SeedPoint` — `src/types/mesh.ts:55-62`, `67-73`.

Corroborating **visual** argument: `SeedMarkers` renders at `position={[p.x,p.y,p.z]}` **inside**
the rotated group (`Viewport.tsx:2751` within `2580`). If stored seeds were world coordinates the
markers would be rotated twice and appear off the mesh; users report them sitting correctly on the
surface. So the stored values must be local.

### 3.2 Stores

| Store | Space | Evidence |
|---|---|---|
| `seedPoints` | local | initialized `appStore.ts:722`; add `723`; remove `727`; type `225`. Transient (not in the persisted whitelist). |
| `suggestedSeeds` | local | `appStore.ts:743-746`; doc explicitly "Transient: NOT persisted" at `:252-257`. |
| `seedEraseMode` | boolean only | `appStore.ts:226-232`, `725`, `728`; doc "Transient on purpose: NOT persisted". |

### 3.3 Consumers of `seedPoints` coordinates

| Consumer | Space it assumes | Verdict | Evidence |
|---|---|---|---|
| `SeedMarkers` render | local | **correct** | `Viewport.tsx:2672-2676` inside the group; `SeedMarker` group `2751`; glyph children `2753-2756` |
| Click-path erase (`onPointerDown` → `eraseNearWorldPoint(local.point)`) | local vs local | **correct** (space-wise) | `1992` with `local` from `getLocalHit` (`1921`), whose `point` is world→local (`1457`) |
| Smear erase (`onPointerMove`) | **passes WORLD, compares to LOCAL** | **BUG** | `2123-2129`, specifically `const hitSmear = raycastFace(...)` (`2125`) and `eraseNearWorldPoint(hitSmear.point)` (`2126`). `raycastFace` returns the raw `THREE.Intersection` (`1791-1802`), whose `.point` is **world**; `eraseNearWorldPoint` (`1076-1111`) compares it to `seedPoints` (local) |
| `seed_grow` IPC | local → backend local | **correct** | `useTauriCommand.ts:503-517`, payload `point: [s.x,s.y,s.z]` at `514`; Rust `SeedInput.point` doc "model-local" `src-tauri/src/segment/seeded.rs:76-81`, snapped against `mesh.vertices` at `seeded.rs:127` |
| `manual_region_add_point` IPC (manual seed click) | local → backend local | **correct** | `Viewport.tsx:2047-2050` sends `local.point`; Rust snap compares against `mesh.vertices` (`src-tauri/src/segment/manual.rs:63-85`, `156`; command at `src-tauri/src/commands/segment.rs:207-227`) |
| Seed panel display | none (uses `.length` only) | **correct** | `src/components/SeedPanel.tsx:950` (`t("seed.count", seedPoints.length)`); no coordinate display |

### 3.4 Rust backend

- `manual_region_add_point` → `snap_point_to_vertex_on_face(mesh, &point, face_index)`
  (`src-tauri/src/commands/segment.rs:215`); the snap compares the incoming point with
  `mesh.vertices[...]` distances (`src-tauri/src/segment/manual.rs:63-85`), i.e. it expects exactly
  the same space as `meshData.vertices` — model-local.
- `seed_grow` consumes `SeedInput { point: [f32;3], face_index }` documented "model-local"
  (`src-tauri/src/segment/seeded.rs:76-81`) and re-snaps with the same function (`seeded.rs:127`).
- `recommend_seeds`, `detect_planar_regions`, `detect_multiview_regions` **emit** local points
  (§3.1). `seed_grow` emits only labels, no coordinates.

**Backend conclusion: the Rust side is uniformly model-local and is not a source of the bug.**

---

## 4. Why R6 / R7 / R8-first missed — numeric reconstruction

All three attempts share one mistake: they fed **local** seed coordinates into **world-space**
camera math. The measured magnitudes fall out of §2 directly.

### 4.1 R6 — `20088d3` (screen match via `Vector3.project`)

Code at that commit: `eraseAtScreen(e.clientX, e.clientY)`; inside, `v.set(s.x,s.y,s.z).project(camera)`
then `sx=(v.x*.5+.5)*rect.width`, `d = hypot(px-sx, py-sy)`, threshold 26 px
(`git show 20088d3 -- src/components/Viewport/Viewport.tsx`). Two errors:

1. **Space:** `project()` treats the local seed as a world point. The rendered marker is at `R·p`,
   so the compared screen positions differ by the projection of `R·p − p`, magnitude
   `≤ zoom · √2·ρ_yz` (the perpendicular component is what shows on screen). With `zoom ≈ 3.5–5.1`
   and `√2·ρ = 46–130 wu`, that is **160–660 px** — the observed 244 px sits inside that range
   (near its low end, consistent with a seed close to the local X axis).
2. **Frame:** `px` was window-absolute while `sx,sy` were canvas-relative, adding a constant
   `≈ hypot(rect.left, rect.top)` ≈ `hypot(≈52, ≈28)` ≈ **59 px** (layout: Toolbar button 40 px,
   `src/components/Toolbar/Toolbar.tsx:186-200`; left panel padding 8, `src/App.tsx:156-160`;
   menu bar ≈26–30 px, `src/components/MenuBar.tsx:285`). Fixed at `73ec499`.

So R6 ≈ (local-as-world projection error, dominant) + (~59 px frame offset, secondary).

> **Refutation of the handoff's R6 interpretation.** The handoff (`:127`) records "click hit 3D point
> vs seed 3D point <2 wu, projected screen difference 244 px" and concludes `project()` deviates
> systematically from the render pipeline. If the two 3-D points are <2 wu apart, their projections
> can differ by at most ≈2·zoom ≈ 8–10 px. The 244 px therefore **cannot** be a projection
> disagreement between two nearby points; it is the wrong input space (§2) plus the frame offset.

### 4.2 R7 — `73ec499` + `7b080e9` (offset fixed, still miss)

`73ec499` subtracts `rect.left/top`, eliminating error (2). The residual is error (1):
`project(local seed as world)` vs the real click pixel. Prediction: `zoom · √2·ρ_yz`.
Observed: **235–652 px** (handoff `:128`). Solving for `ρ`: at `zoom 5.1`, `ρ = 33–90 wu`
(displacement 46–128 wu); at `zoom 3.5`, `ρ = 47–131 wu` (displacement 67–186 wu). Both give
displacement **0.2–0.9 × the 206 wu model diagonal** — i.e. exactly the size of a rotation
displacement, and it is fixed per seed (hence "does not vary linearly with click position").

### 4.3 R8-first — `b98e869` (picking-ray `distanceToPoint`)

The shipped formula (preserved verbatim in HEAD's dead code) is
`dPx = raycaster.ray.distanceToPoint(seedVec) / worldPerPx` with `worldPerPx =
(top−bottom)/zoom/height = 1/zoom` (`Viewport.tsx:1950-1958`). `raycaster.ray` is world space.
Worse, for an OrthographicCamera three.js builds the ray by unprojecting
`NDC z = (near+far)/(near−far)`; solving the ortho NDC-z relation
`z_ndc = −2 z_view/(far−near) − (far+near)/(far−near)` gives `z_view = 0`, so
**`ray.origin` is the camera position**, and `Viewport.tsx:194` sets
`viewDist = maxDim × 10`. Hence:

```
dWorld = distance(localSeed, worldRay) ≤ |localSeed − camera| ≈ 10·maxDim + extent
```

i.e. the metric is bounded by **ten model sizes**, not by the seed's proximity to the click.
This explains everything in the handoff (`:129`):

- lower end `549 px`: `549 / zoom`. At `zoom ≈ 4.1`, `549 px ≙ 134 wu` — the handoff's "**~133 wu**"
  and the §2 rotation displacement are the *same number*. ✳
- upper end `13293 px`: the same metric once the local point is far from the click ray and the
  camera-distance term dominates. `13293/zoom` gives thousands of wu at the auto-fit zoom
  (`≈3240 wu` at `zoom 4.1`) but lands on the camera-distance bound `11·maxDim ≈ 1310 wu`
  (`maxDim≈119` for the 206 wu model) for a zoom of `≈10`, i.e. the user zoomed in ~2.5× beyond
  fit — which is exactly what one does to hit a ~1–3 px dot. Either way the reported value is set
  by `10·maxDim` and the zoom, not by any physical seed-to-click distance, so the handoff's
  "grows with pan / garbage" description is correct. The user must zoom in to hit a tiny marker,
  which inflates `dPx` further. "**Grows with pan**" is the signature of a metric anchored at the
  camera.
- The handoff's conclusion "the ray data itself is garbage" is right about the *result*, wrong
  about the *cause*: the ray is fine; it was pointed at a point in a different space.

✳ The 133 wu coincidence is the strongest single quantitative link: `549 px` at the fit zoom
equals the rotation displacement, not a random distance.

### 4.4 What this refutes in the handoff's "deep lesson"

Handoff `:132-134` says three.js camera APIs (`project`, `setFromCamera`+`distanceToPoint`) are
"unreliable in this orthographic scene, root cause unknown — do not build a third screen-space
scheme." The code evidence refutes that lesson:

- `raycaster.setFromCamera` is used correctly and continuously by every **proven** sow / paint /
  hover hit (`Viewport.tsx:1432`, `1799`, `2255`) — it is the mechanism that makes seeding work at
  all.
- `Vector3.project` does not appear in **any** live code path at HEAD (grep: only comments plus the
  dead block at `1958`/`1980`). The R6/R7 "projection error" was a wrong-space input, not a
  projection defect.
- Therefore the correct lesson is: **every consumer must convert a raycast hit to model-local
  before comparing it to a stored seed** (`getLocalHit` already does exactly that at `1457`).

---

## 5. Live space-mixing inventory of `Viewport.tsx` (task item 4)

| # | Site | Input space | Compared/used against | Verdict |
|---|---|---|---|---|
| 1 | `getLocalHit` `1424-1463`, conversion `1457` | world hit → `worldToLocal` | returned as `LocalHit.point` | **correct** |
| 2 | Lasso click `1491-1494`, `1528` | `LocalHit.point` (local) | backend local + `closeThreshold` (local, `1036-1046`) | **correct** |
| 3 | Brush cursor `1806-1814` (`1811`) | world hit → `worldToLocal` | `hoverInfoRef`; ring rendered inside group (`2600-2603`) | **correct** |
| 4 | Seed click erase `1992` | `local.point` (local) | stored seeds (local) | **correct (space)**; tolerance tight, §6/§9 |
| 5 | Suggested-seed accept `2005-2016` (`2010`) | `local.point` | `suggestedSeeds` (local) | **correct** |
| 6 | Manual add IPC `2047-2050` | `local.point` | backend (local) | **correct** |
| 7 | **Smear erase `2123-2129` (`2126`)** | **world** `hitSmear.point` | stored seeds (**local**) | **BUG — the one live mixing site** |
| 8 | Dead `eraseAtScreen` `1931-1990` (`1939` def, `1950-1963` ray math, `1982-1988` miss) | local seed fed to a **world** ray | — | **dead code** (defined at `1939`, never referenced elsewhere — grep `eraseAtScreen` returns only that line). Must be deleted: it encodes the R8-first assumption and will mislead the next round |
| 9 | Billboard `2786-2792` (`2789`) | camera quaternion in world | child of rotated group | **orientation bug only** — see §6 |
| 10 | `dotSize` `1047-1052` | local glyph size | `LassoOverlay` (`2655-2663`, inside group) | **correct** |
| 11 | Fill / brush paint `2101-2117`, `2130-2142` | `hit.faceIndex` only | face index | **space-free, correct** |
| 12 | Hover highlight `2255-2290` | `hits[0].faceIndex` only | segment labels | **space-free, correct** |
| 13 | `BoundaryLines` `2678-2679` | Rust boundary edges (`mesh.vertices`, local) | rendered inside group | **correct** |

So the smear path is the only *live* mismatch; the click path is already space-consistent.

---

## 6. The billboard bug (task item 5)

Code (`Viewport.tsx:2750-2758`, `2786-2792`): `<group position={pos}>` (anchor) → `<group ref={ref}>`
(child) → glyph meshes; one `useFrame` does `g.quaternion.copy(cam.quaternion)` on the **child**
(`2789`).

- **World orientation of the glyph** = `R_parent · q_child = R · cam.quaternion`. A true billboard
  needs `cam.quaternion`. The error is therefore the **constant** rotation `R = Rx(-π/2)` — a 90°
  error (in the parent frame), independent of the camera pose for any camera whose up ≈ world Y.
- **Computed** (script): the stem's local `+Y` maps to world `R · camUp`. The angular error `ε`
  satisfies `cos ε = up_x²`; hence `ε = 90.0°` for both the front view (`up=(0,1,0)`,
  `R·up=(0,0,−1)`) and a tilted orbit pose, and `0°` only for a 90° camera roll.
- **Consequence:** the stem/leaves (2·size and above) point into the screen / into the surface
  (`R·(0,1,0) = (0,0,−1)`, away from a camera on the +Z side). For a committed (non-ghost) seed the
  material has `depthTest: true` (`2717`), so those parts are occluded by the mesh. What remains
  visible is the sphere — rotationally symmetric, so its screen appearance is unaffected by the
  billboard error.
- **Position:** the anchor is the parent `<group position={pos}>` (`2751`); the child quaternion only
  rotates the child's own geometry about that anchor. **The marker's world position is `R·pos`
  regardless of the billboard bug.** It does **not** move the seed and does **not** change the
  click→surface hit test.
- **Effect on aiming:** it makes the marker read as a ~`0.66 wu` (≈2.6 px at zoom 4) dot instead of
  a sapling, so the user's target is the sphere. This is an *appearance/aimability* bug, not a
  position bug; fixing the orientation would make the glyph larger and easier to hit, but would not
  by itself change erase matching.

---

## 7. "Clicking the red marker produced no log at all" — silent vs noisy returns

(Extra question from the Lead.) Order of evaluation in `onPointerDown` (`Viewport.tsx:1816-2118`):

| Guard | Line | Feedback | Reachable while a red marker is visible? |
|---|---|---|---|
| `e.button !== 0` | `1817` | **SILENT** | yes (right/middle click) |
| `isPointerInGizmo(...)` | `1820` | **SILENT** | yes (click inside the gizmo corner) |
| `_altHeld` | `1822` | **SILENT** | yes (Alt held) |
| View-tool Ctrl branch | `1827-1842` | noisy (status/log) | no — requires `isViewTool` |
| Ctrl/Cmd merge branch | `1850-1862` (`1851` silent if no `segmentLabels`, `1853` silent off-model, `1858` noisy) | mixed | **yes** — a Ctrl+click on a red marker silently does nothing (or shows merge hints) |
| `if (isViewTool) return` ×3 | `1866`, `1869`, `1870` | **SILENT** | no — markers only render under `isSeedTool` |
| auto-mode guard `isSeedTool && segmentMode==="auto" && !seedPickMode` | `1874-1877` | noisy (`seed.autoNoSowing`) | **yes** — see below |
| seed-pick sub-mode | `1883-1903` | noisy | mutually exclusive with erase (`SeedPanel.tsx:461-472`) |
| `isSeedTool` → `getLocalHit` null | `1904-1912` (lasso), `1921-1926` (seed) | noisy: logs `seed click missed model` (`1924`) | yes, but it logs |
| **erase armed** | `1938` → `1992` | always logs (`1092` hit / `1100` miss) + status | yes |

**Reachability of the erase branch.** The markers are rendered by
`{isSeedTool && <SeedMarkers … erase={seedEraseMode}/>}` (`Viewport.tsx:2672-2676`), and the erase
branch lives inside `if (isSeedTool)` (`1913`) guarded by `if (seedEraseMode)` (`1938`). Also
`setActiveTool` disarms both `seedPickMode` and `seedEraseMode` on every tool change
(`src/store/appStore.ts:640-648`, `seedEraseMode: false` at `646`), and the erase toggle itself is
only rendered in manual mode (`src/components/SeedPanel.tsx:947`). **Therefore: red markers visible
⇔ `activeTool === "seed"` ∧ `seedEraseMode === true` ∧ manual mode at the time of arming — so the
branch is structurally reachable and a plain left-click cannot skip it.**

**Verdict on the Lead's two candidate causes:** the code evidence supports **the coordinate-space
cause**, not "the handler never executes".

1. R7's measured `235–652 px` is computed *inside* the erase branch — it proves the branch ran.
2. R6's "no HUD log" is explained without invoking a skipped handler: `git show 20088d3` shows the
   R6 erase branch contained **only** `removeSeedPoint` + `setStatusMessage` — **no `log.info`**
   (the `[seed-erase]` probe was added by `7b080e9`, i.e. R7). The generic
   `seed tool: pointer down` log existed (`20088d3:1892`), so a completely empty HUD would be
   anomalous, but the *erase-specific* absence is expected. (For the record, the R7 probe was
   shipped with the wrong call signature — `log.info(tag, object)` instead of
   `log.info(tag, message, data?)`, `src/utils/logger.ts:73-77` — so it renders as
   `[object Object]` via the `fmt` template at `logger.ts:57-64`; handoff `:150-151` warns about
   exactly this. It still *executed*, which is all the argument needs.)
3. The only silent early returns require a special input state (non-left button, gizmo corner, Alt,
   Ctrl+click). None of them is the default single-click flow.

**Two genuine upstream hazards worth keeping on the list (secondary, not the main cause):**

- **Auto-mode guard ordering:** `1874` sits *before* the erase branch (`1938`). The erase toggle is
  only shown in manual mode, but `setSegmentMode` does **not** reset `seedEraseMode`
  (`appStore.ts:732-740`). So a user who arms the eraser in manual mode and then switches the seed
  tool to **auto (fuse)** keeps red markers on screen, and clicking them does nothing except show
  `seed.autoNoSowing` — no erase, no `[seed-erase]` log. This is a real reachable "clicking the red
  dot does nothing" state.
- **Ctrl+click** on a red marker never erases and can return silently (`1850-1853`).

---

## 8. Recommended correction (Lead owns the fix; listed, not implemented)

1. **Smear path (`2126`) is the live bug:** every erase entry point must hand
   `eraseNearWorldPoint` a **model-local** point. Either route the smear through `getLocalHit`
   (which already does `worldToLocal` at `1457`), or convert the raw intersection once:
   `meshRef.current.worldToLocal(hitSmear.point.clone())` before the call.
2. **Delete the dead `eraseAtScreen` block (`1931-1990`).** At HEAD it is defined and never called
   (grep `eraseAtScreen` → only line `1939`); it carries the refuted "local seed vs world ray"
   assumption (`1950-1963`, `1980-1988`) and will be re-adopted by a future round otherwise.
3. **Billboard (`2789`):** copy the camera's **world** quaternion into the child, i.e. compose the
   parent's inverse world quaternion (or set it from `camera.getWorldQuaternion`) so the glyph
   actually faces the camera. This is cosmetic/aimability, but it also makes the marker's visible
   size match the hit target.
4. **If erase should work in auto mode**, move the `1874` guard after the erase branch (or exempt
   `seedEraseMode`); otherwise document the restriction to the user.
5. No coordinate-system change is needed on the click path (`1992`) or in the Rust backend (§3.4).

---

## 9. Still unresolved

1. **Runtime numbers were never captured.** The user's failing sessions did not record the STL
   bbox, canvas size or `camera.zoom`. Every px/wu figure above is derived from
   `Viewport.tsx:1059-1066` (size → D ≈ 206 wu), `203-206` (zoom) and the one measured anchor
   (`549 px ↔ ~133 wu`). One HUD line at erase time printing `{zoom, rect, seedMarkerSize, dWorld}`
   would pin them; that is a diagnostic request, not a fix.
2. **Was `df094b7` ever user-tested?** Handoff `:10-13` and `:130` say the world-distance scheme is
   "not yet accepted". The click path is space-consistent in code, so the historical "never erased"
   reports must come from R6/R7/R8-first builds. If the eraser *still* misses on `df094b7`, the
   remaining suspect is the **geometric tolerance**, not the space: threshold `0.825 wu ≈ 3 px` at
   the auto-fit zoom, and the surface hit is not the seed (the seed is the face centroid, the hit is
   the clicked triangle point — on a tilted surface the 3-D distance is `screen offset / cos(tilt)`).
3. **`worldToLocal` assumes a current `matrixWorld`.** It does not call `updateWorldMatrix` (three.js
   `Object3D.worldToLocal` mutates via `matrixWorld.invert()`); R3F updates matrices per render, so
   this is fine for normal pointer events. If a seed were ever added before the first render, it
   would be stored in world coordinates and its marker would then appear double-rotated off the
   surface. No evidence for this in the logs; flagged for completeness.
4. **The billboard conclusion is computed, not observed.** No screenshot of the rendered marker was
   available; the "marker looks like a dot, not a sapling" claim follows from `R·(0,1,0)=(0,0,−1)`
   plus `depthTest: true` (`2717`), not from a captured image.
5. **R8-first's exact probe code is not in the repo.** It was rewritten at `df094b7`; the
   reconstruction in §4.3 uses the identical formula preserved in the dead block
   (`1950-1958`, `1980`) plus the handoff's reported `549 → 13293 px` / `~133 wu`.
6. **Eye-region seeds cannot be erased by construction.** `eyeSeeds` (centroids from `faceIndices`)
   are built only at grow time (`SeedPanel.tsx:781-812`, `829`) and never enter `seedPoints`; the
   eraser only walks `seedPoints` (`Viewport.tsx:1081-1089`). If a user expects to erase an
   eye-derived seed in the viewport, no coordinate fix will help.
