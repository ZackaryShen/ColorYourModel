# Algorithms

How CYM understands and partitions a mesh — the math and the principles.
For engineering mechanisms (picking, highlight, undo/redo, export), see [../technical/](../technical/).

**Status:** skeleton files with scopes; content is being written.
**Rule of thumb:** the source code (`src-tauri/src/segment/`) is the authority; the numbered Chinese docs (`../07…10`) are historical research records.

| Document | Scope | Status |
|----------|-------|--------|
| [segmentation.md](segmentation.md) | auto-segmentation: three-phase dihedral pipeline + v2 unified interface with 8 algorithms | ✍️ pending |
| [seed-grow-fuse.md](seed-grow-fuse.md) | seed recommendation, manual grow, auto fuse + tiny-region merge | ✍️ pending |
| [eye-detection.md](eye-detection.md) | eye-region semantic detection (global, template-based) | ✍️ pending |

Paper references live in [`../08-智能分区-算法参考与论文.md`](../08-智能分区-算法参考与论文.md).
