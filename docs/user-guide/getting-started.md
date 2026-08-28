# Getting Started

> Part of the CYM wiki. Start here if you want to build and run ColorYourModel.

## What you need

- [Node.js](https://nodejs.org/) ≥ 18 (frontend toolchain)
- [Rust](https://www.rust-lang.org/tools/install) ≥ 1.70 + Cargo (backend)
- [Tauri 2 system prerequisites](https://v2.tauri.app/start/prerequisites/) — on Windows this is mainly WebView2

## Build & run

```bash
git clone https://github.com/ZackaryShen/ColorYourModel.git
cd ColorYourModel

npm install
npm run tauri dev      # first run compiles the Rust backend (~2-3 min)
```

The dev build gives you hot reload for the frontend; incremental Rust builds are fast after the first compile.

For a packaged release build, use `npm run tauri build` (produces the installer for your platform).

## Your first colour job

1. **Import** an STL white model (binary or ASCII). 1.5M-face-class models load in ~4 s with a progress bar.
2. **Segment** — an automatic dihedral-angle segmentation (30° threshold) runs right after import; open **Intelligent Segmentation** to switch algorithm or tune parameters (see [Auto Segmentation](auto-segmentation.md)).
3. **Refine regions** — place seeds and grow/fuse (see [Seed Tools](seed-tools.md)), or merge/split/rename regions in the Segments panel.
4. **Paint** — pick a tool and go (see [Painting Tools](painting-tools.md)). Everything is undoable.
5. **Export** a 3MF with machine presets, or an OBJ with per-face colours (see [Exporting](exporting.md)), and slice it in Snapmaker Orca / OrcaSlicer.

```mermaid
flowchart LR
    A["Import STL"] --> B["Segment<br/>(auto + seeds)"]
    B --> C["Refine regions"]
    C --> D["Paint"]
    D --> E["Export 3MF / OBJ"]
    E --> F["Slice & print"]
```

## Verification & tests

```bash
npx tsc --noEmit               # TypeScript
npx vitest run                 # frontend unit tests
cd src-tauri && cargo test --lib   # backend unit + regression tests
```

## Debug logging

```bash
# Windows PowerShell
$env:RUST_LOG="debug"; npm run tauri dev

# Linux / macOS
RUST_LOG=debug npm run tauri dev
```

Frontend exceptions are captured by the JS bridge and viewable in the in-app **DebugLogViewer** (see [Crash Diagnostics](../technical/crash-diagnostics.md)).

## Troubleshooting

| Symptom | What to check |
|---------|---------------|
| First run takes minutes | Normal — Rust compiles from scratch; later runs are incremental |
| Model fails to load | Check the log; UV-sphere-style meshes with many coincident-axis vertices are supported since the kdtree fix (`69a17ea`) |
| UI language is English by default? | No — the app starts in Chinese; switch in the UI, the choice is persisted (see [i18n note](../developer/development.md#i18n)) |
