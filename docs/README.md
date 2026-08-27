# Documentation Index

This directory mixes two kinds of documents. When they disagree, **the source code is the authority**.

| Kind | Where | Language |
|------|-------|----------|
| Deep dives — algorithms | [`algorithms/`](algorithms/) | EN skeleton, content in EN or 中文 |
| Deep dives — engineering mechanisms | [`technical/`](technical/) | EN skeleton, content in EN or 中文 |
| Case showcase & verification records | [`cases/`](cases/) | EN or 中文, template provided |
| Numbered working documents | `01…10-*.md` | 中文 |

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

> Note: 07/08/09/10 are historical research records. For current behaviour, read the code (`src-tauri/src/segment/`) and the deep-dive stubs under `algorithms/`.

## Iteration journal

`loop-journal.md` — per-iteration retrospectives of the adversarial development loop.
