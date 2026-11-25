# Texture Painting Implementation Plan

> **For agentic workers:** Use superpowers:subagent-driven-development or superpowers:executing-plans. Use focused red-green tests and independent review. This plan tracks work on the existing feature branch.

**Goal:** Convert embedded GLB textures into editable face painting and retain user-confirmed filament mappings through 3MF export and CYM reopen.

**Architecture:** Rust validates, subdivides and bakes GLB data. The webview renders synchronized preview geometries and applies a versioned candidate on confirmation. 3MF export derives face tool slots and project metadata from the same confirmed plan.

**Tech Stack:** Rust, Tauri, glTF 2.0, PNG/JPEG, React, Three.js, Vitest.

**Spec:** [Texture painting](../specs/2026-11-20-texture-painting-design.md) and [approved UI](../prototypes/texture-painting-ui-prototype.html).

## Global Constraints

- Branch `feature_texture_mapping`, review base `b996232`.
- Single embedded opaque base-color PNG/JPEG with UVs, static triangulated GLB. Color count 1–16.
- Keep the active model during preview, errors and cancellation. Reuse CYM themes and editing.
- Actual target slots constrain export. Keep manual assignments by color and write package settings from the same slot snapshot.
- Reuse 0.2 engineering format and validate geometry before restoring private state.

## Review Focus

- Geometry and UV have distinct identities at seams. Bounded input validation happens before decode or growth.
- Out-of-order preview, cancel and apply cannot replace the current model.
- Texture color/slot/count can diverge after a stroke, fallback, target switch or theme change.
- Export must represent stable 1-based slots even when several colors share a slot or slot 16 is used.
- Reopen must preserve pre-quantized editable face colors, history and confirmed assignments without the GLB.

## Work and Evidence

- [x] Validate GLB inputs and bake at most 16 colors. Exercise orientation, UV seams, wrapping, explicit nearest filtering, and subdivision budgets.
- [x] Add cancellable candidates and connect file selection and drag-drop to the original and converted preview in both themes. Keep the live model when preparation fails or a job expires.
- [x] Confirm explicit color-to-slot mappings. Use a replaceable predictor for suggestions, and share the same face groups between preview and 3MF. Keep active color anchors when additional edited RGBs must be grouped into at most 16 export rows.
- [x] Reopen the engineering 3MF through the native import path. Restore exact face colors, history, and mappings after verifying the geometry fingerprint.
- [x] Run integrated Rust and frontend tests, build the app, perform native UI and Snapmaker Orca slicing checks, and resolve the findings from independent spec and code reviews.

## Acceptance Evidence

- macOS WKWebView and native Rust passed the GLB palette, theme switching, invalid UV, cancellation, U1 mapping, export, and reopen flows. Reopen restored 49,152 faces and all 16 assignments after the source GLB was moved. A 12-face binary STL still loaded and supported a brush stroke followed by Undo. The harness supplied file picker paths. After the Mac unlocked, a visible WKWebView rendered both preview panes in both themes with normal frame scheduling.
- The native texture brush applied a 50% stroke across 53 faces. Recoloring preserved the underlying texture within two RGB levels per channel, touched only those faces, and passed exact undo and redo checks.
- The installed Snapmaker Orca 01.10.01.50 sliced the 16-color U1 archive. A separate cuboid assigned four source colors per wall to a single slot. The verifier checked 90 middle layers per wall and observed only the expected tool on each wall. The slicer emitted a thumbnail shader error and produced valid G-code. `tools/generate_texture_fixture.py --sides-by-slot` and `tools/verify_texture_toolpaths.py` reproduce the geometry and assertions.
- Rust library tests passed 346 (20 ignored). Vitest passed 183 across 27 files. The frontend and native macOS development builds succeeded. Independent review repeated the seven shaped repaint tests and reproduced every input archive entry for the regional slicing sample.
- A standalone bake kept a valid manifold at 196,608 faces. It used roughly 58 MiB peak RSS on the tested Mac. The importer rejected a source mesh above 200,000 faces. This measures the Rust bake process.
- Automatic grouping applies after edited colors exceed 16. Users confirm assignments for the displayed groups. CYM reopens the exact colors and undo stack. The inherited engineering format omits the editor's transient brush-source groups.
- The native test used macOS only. Physical print color calibration is deferred. Suggestions use nominal filament colors through the predictor interface.

## Complex full-mesh acceptance

- The source bust STL has 148,460 triangles. A four-material atlas with four shades per material paints the hair and frames, face and hand, suit, and shirt. CYM applied the full mesh, saved all 148,460 expected RGBA and slot values, and reopened all 16 mappings after the input copy of the GLB was moved. The geometry differs from the original STL by at most 0.000009 mm per coordinate due to float conversion.
- Snapmaker Orca 2.4.2 build 01.10.01.50 cut 750 layers for a U1 with a 0.4 mm nozzle and the 0.20 Standard process. The initial cut warned about floating regions. A second cut with automatic tree supports generated support and interface paths, retained every triangle's tool code, and removed the floating-region warning.
- A geometric audit of the tree-support G-code compared clear outer-wall segments to the colored surface after applying the slicer's placement transform. It matched 263,552 of 263,567 segments. All audited suit and shirt segments used T2 and T3 respectively. The 15 mismatches were localized near the head-side hairline and hand-to-sleeve transition. The audit excludes short segments, first and last layers, near-horizontal surfaces, and ambiguous or distant surface matches. Orca's GUI completed the tree-support cut and displayed the filament preview. Physical printing remains unverified.
