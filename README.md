<p align="center">
  <img src="assets/icon.png" alt="ColorYourModel logo" width="128">
</p>

# ColorYourModel

[English](README.md) | [简体中文](README.zh-CN.md)

> Turn white-model STLs into region-based, multi-colour 3MFs ready for multi-material 3D printing.

[![License: AGPL-3.0](https://img.shields.io/badge/License-AGPL--3.0-blue.svg)](LICENSE)
[![Release](https://img.shields.io/badge/release-v0.1.0-blue)](https://github.com/ZackaryShen/ColorYourModel/releases/tag/v0.1.0)
![Built with Tauri 2](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)
![React 18](https://img.shields.io/badge/React-18-61DAFB?logo=react&logoColor=black)

**ColorYourModel (CYM)** is a desktop app (Tauri 2 + React + Three.js frontend, Rust backend) that solves a practical 3D-printing problem: white-model STL files are just a pile of unstructured triangles — no "regions", no colour. Painting such a model triangle-by-triangle in a slicer is hopeless for anything detailed.

CYM automatically segments the mesh into semantic regions (helmet, skin, base…), lets you refine and paint them with region-aware tools, and exports a standards-compliant **3MF with per-region colours** that imports into Snapmaker Orca / OrcaSlicer as real filament assignments — verified end-to-end on real hardware (Aug 2026, Snapmaker U1).

```mermaid
flowchart TD
    A["STL white model"] --> B["Segment — Seed panel<br/>auto fuse (planar / multiview / dihedral / eye vote)<br/>or manual seed grow · eye detection"]
    B --> C["Refine<br/>merge / split / rename / resegment"]
    D --> E["Painting<br/>brush · spray · smart brush · fill · eraser · picker<br/>unified undo & redo"]
    E --> F["Export<br/>3MF (colour + machine presets) · OBJ (per-face colours)"]
    F --> G["Snapmaker Orca / OrcaSlicer<br/>slice & print"]
```

> **New here?** Start with the **[User Manual](docs/bak/user-guide/manual.md)** (简体中文) · **[English manual](docs/bak/user-guide/manual.en.md)** — every button, shortcut, camera trick, segmentation workflow, export step, and the current limitations, explained for first-time users.

## Screenshots

| | |
|---|---|
| <img src="samples/sanji-diorama/example02.png" alt="1.87M-face figure segmented into 37 regions" width="480"> | <img src="samples/liangsheng/example01.png" alt="Bust painted as 11 regions" width="480"> |
| 1.87M-face figure → 37 regions, ready for per-part colour | Painted bust — this model drove the end-to-end 3MF verification |

Thirteen real runs — sculpts, AI-generated meshes, signage, architecture — live in the [**Examples Gallery**](docs/bak/cases/examples.md).

## Download

Ready-to-run installers live on the [**releases**](https://github.com/ZackaryShen/ColorYourModel/releases) page — current: [v0.1.0](https://github.com/ZackaryShen/ColorYourModel/releases/tag/v0.1.0).

| Platform | Installer |
|----------|-----------|
| Windows | [`x64-setup.exe`](https://github.com/ZackaryShen/ColorYourModel/releases/download/v0.1.0/ColorYourModel_0.1.0_x64-setup.exe) (recommended) · [`x64_en-US.msi`](https://github.com/ZackaryShen/ColorYourModel/releases/download/v0.1.0/ColorYourModel_0.1.0_x64_en-US.msi) |
| Linux | [`amd64.deb`](https://github.com/ZackaryShen/ColorYourModel/releases/download/v0.1.0/ColorYourModel_0.1.0_amd64.deb) · [`x86_64.rpm`](https://github.com/ZackaryShen/ColorYourModel/releases/download/v0.1.0/ColorYourModel-0.1.0-1.x86_64.rpm) · [`AppImage`](https://github.com/ZackaryShen/ColorYourModel/releases/download/v0.1.0/ColorYourModel_0.1.0_amd64.AppImage) (bundles WebKit, ~80 MB) |

> No macOS builds yet (no signing identity). The UI is Chinese-first, English is one setting away. To build from source, see [Getting started](#getting-started).

## Demo videos

| Demo | What it shows |
|------|---------------|
| [Segmentation & export](https://github.com/ZackaryShen/ColorYourModel/releases/download/media/partition-and-export-demo.mp4) | Seed-based fuse segmentation, then per-region 3MF export |
| [Painting tools](https://github.com/ZackaryShen/ColorYourModel/releases/download/media/painting-tools-demo.mp4) | Brush / spray / fill painting on the segmented model |
| [3MF showcase](https://github.com/ZackaryShen/ColorYourModel/releases/download/media/3mf-showcase-demo.mp4) | The exported multi-color 3MF loaded in Snapmaker Orca, regions mapped to filament assignments |

> Videos are hosted as [release assets](https://github.com/ZackaryShen/ColorYourModel/releases/tag/media) — they stream in place and never bloat the git clone.

## Features

### Working today ✅

**Import & segmentation**

- **STL import** — binary + ASCII, ~4 s for 1.5M-face-class models, with progress events
- **Segmentation** — SeedPanel one-click **fuse** (planar / multiview / dihedral / eye four-channel vote) or manual seed growing; 8 interchangeable algorithms behind one backend interface, exposed per-region through the SegmentsPanel resegment picker. Import does **not** auto-segment — region generation is an explicit step
- **Seed-based segmentation** — recommended seeds (planar / multiview / cross-section / saliency), point-by-point manual grow, and auto fuse with tiny-region merging
- **Eye-region detection** — one-click detection for figure models, global (no ROI needed)
- **Region management** — merge / split / rename regions, resegment a single region

**Painting**

- **Paint tools** — brush, spray, smart brush (region-boundary aware), fill, eraser, colour picker
- **Label-authority fill** — fill targets strictly the region you clicked, so two regions can safely share the same colour (regression-tested)
- **Unified undo / redo** — one timeline covering paint, fill, erase and manual edits

**Export**

- **Export 3MF** — verified end-to-end (2026-08): multi-filament colours import correctly into Snapmaker Orca; embedded machine preset makes the slicer show *Snapmaker U1 (0.4 nozzle)* out of the box. Export dialog: machine → nozzle → process → filament slots → target slicer, with multi-region selection and apply-all
- **Export OBJ** — per-face colours via MTL material groups, quantized to at most 256 colours

**Viewport & ergonomics**

- **Segment view** — region-coloured visualization with boundary outlines; hover highlight is a GPU shader (per-vertex label attribute, O(1) switch, on-demand rendering)
- **BVH face picking** — `three-mesh-bvh` accelerated CPU raycasting; stable hits on 1.5M-face meshes
- **Ergonomics** — 3D brush-cursor ring, Space-to-pan, progress bars, crash diagnostics (JS error bridge)
- **i18n** — Chinese (default) and English UI, choice persisted

### Planned 🗓

- [ ] Colour palette presets + colour history
- [ ] Batch processing of similar models
- [ ] OrcaSlicer plugin-form integration

## Tech stack

| Layer | Tech | Notes |
|-------|------|-------|
| Frontend | React 18 + TypeScript | declarative UI |
| 3D | Three.js + @react-three/fiber + three-mesh-bvh | rendering + accelerated picking |
| State | Zustand (persist) | app state + preferences |
| Desktop | Tauri 2 | native window + Rust backend |
| Backend | Rust | mesh I/O, segmentation, painting, export |
| Rust crates | nalgebra · parry3d · petgraph · kiddo · stl_io · quick-xml · zip | geometry, graphs, spatial index, parsing |
| Build/Test | Vite 6 · Vitest + Testing Library · cargo test | `tsc --noEmit` / `vitest run` / `cargo test --lib` |

### Architecture

```mermaid
flowchart LR
    subgraph FE["React + TypeScript"]
        UI["Toolbar · Viewport<br/>SeedPanel · ExportDialog<br/>SegmentsPanel · …"]
        STORE["Zustand store<br/>(persisted prefs)"]
        UI <--> STORE
    end
    subgraph BE["Rust backend (Tauri 2)"]
        CMD["commands/<br/>mesh · segment · paint<br/>history · export · js_bridge"]
        SEG["segment/<br/>8 algorithms + seeds/fuse/eye"]
        CORE["mesh/ · paint/ · export/"]
        CMD --> SEG
        CMD --> CORE
    end
    UI -- "Tauri invoke (IPC)" --> CMD
```

## Project structure

```
ColorYourModel/
├── src/                          # React frontend (TypeScript)
│   ├── components/
│   │   ├── Toolbar/              # tool selection + parameters
│   │   ├── Viewport/             # 3D viewport (BVH picking, shader highlight)
│   │   ├── BrushSettings/        # brush parameters
│   │   ├── ColorPanel/           # colour selection
│   │   ├── SegmentsPanel/        # region management
│   │   ├── StatusBar/            # status + HUD diagnostics
│   │   ├── ExportDialog/         # 3MF export wizard (presets)
│   │   ├── SeedPanel.tsx         # seed segmentation: Auto (fuse) / Manual (grow)
│   │   ├── IntelligentSegmentPanel.tsx  # auto-segmentation algorithms + params
│   │   └── DebugLogViewer.tsx    # in-app crash diagnostics
│   ├── hooks/                    # useMesh · usePaintTool · useTauriCommand · useHistory
│   ├── store/appStore.ts         # Zustand global state (persisted)
│   ├── types/                    # mesh · segment · export types
│   ├── utils/                    # logger · segment palette
│   ├── i18n.ts                   # zh (default) / en
│   ├── App.tsx · main.tsx
├── src-tauri/                    # Rust backend
│   ├── src/
│   │   ├── commands/             # Tauri IPC layer (mesh/segment/paint/history/export)
│   │   ├── mesh/                 # STL loader · model + adjacency · kdtree
│   │   ├── segment/              # segmentation algorithms, seeds, fuse, eye detection
│   │   ├── paint/                # brush · spray · fill · smart snap · eraser
│   │   ├── export/               # 3MF · OBJ · slicer presets · quantize
│   │   └── lib.rs                # Tauri builder + command registry
│   ├── resources/presets/        # generated slicer presets (from OrcaSlicer vendor tree)
│   └── Cargo.toml
├── docs/                         # HTML documentation (editable md sources in docs/bak/)
├── tools/                        # preset extraction & 3MF/fill verification scripts
├── examples/                     # sample output (paint_color_sample.3mf)
└── CHANGELOG.md
```

## Getting started

### Prerequisites

- [Node.js](https://nodejs.org/) ≥ 18
- [Rust](https://www.rust-lang.org/tools/install) ≥ 1.70 + Cargo
- [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) (WebView2 etc.)

### Run

```bash
git clone https://github.com/ZackaryShen/ColorYourModel.git
cd ColorYourModel

npm install
npm run tauri dev      # first run compiles the Rust backend (~2-3 min)
```

### Tests & checks

```bash
npx tsc --noEmit               # TypeScript
npx vitest run                 # frontend unit tests
cd src-tauri && cargo test --lib   # backend unit + regression tests
```

### Debug logging

```bash
# Windows PowerShell
$env:RUST_LOG="debug"; npm run tauri dev

# Linux / macOS
RUST_LOG=debug npm run tauri dev
```

## Documentation

📖 **Online docs: <https://zackaryshen.github.io/ColorYourModel/>** — user guide, algorithms, technical notes, examples gallery.

Markdown sources live in [`docs/bak/`](docs/bak/README.md) (mirroring the docs tree); the committed HTML archives they generate are the official, readable pages. Short version:

- **[User Manual / 使用手册](docs/bak/user-guide/manual.md)** (简体中文, [English](docs/bak/user-guide/manual.en.md)) — the complete how-to with screenshots. **Start here.**
- [docs/algorithms/](docs/bak/algorithms/) — segmentation & detection algorithms (math and principles)
- [docs/technical/](docs/bak/technical/) — engineering mechanisms (picking, highlight, undo/redo, export pipeline)
- [docs/cases/](docs/bak/cases/) — case showcase & verification records
- `docs/bak/01…10-*.md` — Chinese working documents (PRD, architecture, defects, roadmap, research notes)
- [CHANGELOG.md](CHANGELOG.md) — per-iteration changelog

The project is developed with an adversarial development loop (`PLAN → REFUTE → REVISE → IMPLEMENT → TEST → RETROSPECT`); see the CHANGELOG for per-round records.

## Roadmap

> [v0.1.0](https://github.com/ZackaryShen/ColorYourModel/releases/tag/v0.1.0) is the first tagged release. Status is measured against PRD acceptance criteria, not "code exists".

- [x] **v0.1 core loop** — STL import → segmentation → painting → verified 3MF export (Aug 2026, released as v0.1.0 Sep 2026)
  - 3MF export verified end-to-end on Snapmaker Orca (U1); fill routing regression-tested
  - Remaining: real-machine re-verification of the fill cluster (see [docs/04](docs/bak/04-缺陷与遗留问题清单.md))
- [ ] **v0.2** — palette presets + colour history, UX polish
- [ ] **v1.0** — batch processing, OrcaSlicer plugin-form integration

## Community

- 💬 [**Discussions**](https://github.com/ZackaryShen/ColorYourModel/discussions) — ask in Q&A, show off your painted models in Show and tell, pitch ideas. 中文 / English both welcome.
- 🐛 [Issues](https://github.com/ZackaryShen/ColorYourModel/issues) — bug reports & feature requests (templates provided: face count / OS / version)
- 🤝 [CONTRIBUTING.md](CONTRIBUTING.md) — build, test commands, docs workflow. Please follow [Conventional Commits](https://www.conventionalcommits.org/); by contributing you agree that your contributions are licensed under AGPL-3.0.

## License

This project is licensed under the [GNU AGPL-3.0](LICENSE).

Copyleft keeps derivatives — including network-service deployments — open under the same terms, while every person and company remains free to use, study, modify and redistribute the software. Note: the bundled slicer presets are extracted from the OrcaSlicer (AGPL-3.0) vendor tree and are treated as AGPL-derived data. For alternative licensing, contact the maintainer.
