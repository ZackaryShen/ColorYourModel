# ColorYourModel Wiki

This is the documentation hub for **ColorYourModel (CYM)** — a desktop app that turns white-model STLs into region-based, multi-colour 3MFs for multi-material 3D printing. When any document disagrees with the code, **the source code is the authority**.

**How the docs work**: markdown sources live in `bak/` — a mirror of the `docs/` tree (this file is `bak/README.md`). The committed `*.html` pages in `docs/` are generated from them and are the official, readable archive. To edit documentation: change the md under `bak/`, run `npm run docs:build`, commit both. CI fails if the HTML drifts out of sync.

```mermaid
flowchart LR
    MD["bak/ markdown sources<br/>(editable)"] -->|"npm run docs:build"| HTML["docs/ HTML archives<br/>(official pages)"]
    HTML --> UG["User Guide"]
    HTML --> AL["Algorithms"]
    HTML --> TE["Technical"]
    HTML --> DV["Developer"]
    HTML --> CA["Cases"]
```

## I want to…

| Goal | Page |
|------|------|
| Build and run the app for the first time | [Getting Started](user-guide/getting-started.md) |
| Understand / tune automatic segmentation | [Auto Segmentation](user-guide/auto-segmentation.md) |
| Refine a figure model (seeds, grow, fuse, eyes) | [Seed Tools](user-guide/seed-tools.md) |
| Paint, fill, undo, navigate | [Painting Tools](user-guide/painting-tools.md) |
| Get a multi-colour printable file | [Exporting](user-guide/exporting.md) |
| Know every Tauri command and signature | [IPC Reference](developer/ipc-reference.md) |
| Contribute code or docs | [Development Guide](developer/development.md) |
| See a real end-to-end result | [3MF verification case](cases/01-3mf-end-to-end.md) |
| Read how an algorithm works inside | [Algorithms](algorithms/README.md) |

## Document map

| Kind | Source (bak/) | HTML archive | Language |
|------|---------------|--------------|----------|
| User guide | [`bak/user-guide/`](user-guide/getting-started.md) | `docs/user-guide/` | EN |
| Algorithms (math & principles) | [`bak/algorithms/`](algorithms/README.md) | `docs/algorithms/` | EN · 中文 content ok |
| Engineering mechanisms | [`bak/technical/`](technical/README.md) | `docs/technical/` | EN · 中文 content ok |
| Cases & verification records | [`bak/cases/`](cases/README.md) | `docs/cases/` | EN/中文 |
| Developer reference | [`bak/developer/`](developer/ipc-reference.md) | `docs/developer/` | EN |
| Numbered working documents | `bak/01…10-*.md` | `docs/` | 中文 |
| Iteration journal | [`bak/loop-journal.md`](loop-journal.md) | `docs/loop-journal.html` | 中文 |

## Numbered series (working documents, 中文)

| # | Document | Content |
|---|----------|---------|
| 01 | [PRD](01-PRD-ColorYourModel.md) | product requirements & acceptance criteria |
| 02 | [技术架构说明书](02-技术架构说明书.md) | technical architecture |
| 03 | [迭代复盘与工程经验沉淀](03-迭代复盘与工程经验沉淀.md) | iteration retrospectives |
| 04 | [缺陷与遗留问题清单](04-缺陷与遗留问题清单.md) | defect tracker (P0/P1/P2 with evidence) |
| 05 | [v0.1 路线图 backlog](05-v0.1-路线图-backlog-2026-08-06.md) | roadmap & backlog |
| 06 | [v0.1 开发路线图](06-v0.1-开发路线图.md) | development roadmap |
| 07 | [智能分区设计](07-智能分区设计.md) | smart segmentation design |
| 08 | [智能分区-算法参考与论文](08-智能分区-算法参考与论文.md) | algorithm references & papers |
| 09 | [连续平面与边界-算法融合研究](09-连续平面与边界-算法融合研究.md) | plane/boundary fusion research |
| 10 | [眼睛区域语义识别](10-眼睛区域语义识别.md) | eye-region detection research |

> Note: 07/08/09/10 are historical research records. For current behaviour, read the code (`src-tauri/src/segment/`) and the wiki pages under `algorithms/`.
