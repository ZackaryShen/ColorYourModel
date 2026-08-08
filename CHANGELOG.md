# Changelog

所有重要变更均记录于此文件。格式基于 [Keep a Changelog](https://keepachangelog.com/)。

---

## [Unreleased]

### Added — 3MF 导出预设库 (2026-08-07, `2e4e070`)

- **导出对话框**：导出前可选 厂商/机型 → 喷嘴直径 → 工艺 → 每槽耗材 → 目标切片器（Snapmaker Orca / OrcaSlicer），级联过滤 + 调色板槽位预览
- **预设库**：`tools/extract_orca_presets.py` 从 OrcaSlicer vendor 树萃取（vendor 索引白名单 + inherits 链解析），编译期嵌入 `snapmaker_u1.json`（4 喷嘴变体 / 30 工艺 / 113 耗材）
- **机器识别**：selected 导出嵌入完整 `machine_settings_1.config`（含 gcode 宏/运动参数），使 OrcaSlicer 下拉框直接显示 `Snapmaker U1 (0.4 nozzle)`，无需用户预装 vendor profile
- **上次选择持久化**：导出配置存 zustand persist（`lastExportSelection`，值域校验）
- 后端新增 IPC：`list_export_presets`、`export_palette_preview`；`export_3mf_command` 支持可选 `selection`

### Added — 智能分区前端面板（2026-08-07, 本轮提交）
- **统一分区入口**：新增 `IntelligentSegmentPanel`（模态，沿用 `ExportDialog` 的 overlay/dialog 样式）。算法 `<select>`（曲率 K-Means / 形态直径 SDF / 二面角）+ 动态参数滑块（聚类数 k、法线平滑迭代、融合 SDF 开关、棱角阈值、二面角阈值），运行即 `invoke("auto_segment_v2", { algorithm })`。
- **偏好持久化**：`appStore` 新增 `lastAlgorithmParams`（按算法分桶保留已调滑块）+ `lastSegmentKind`，走 zustand persist 白名单 + `sanitizeAlgorithmParams` 值域校验；面板运行即写回 localStorage。
- **收口硬编码**：工具栏导入后的自动分区改用 `autoSegmentV2(buildAlgorithm(持久化 or 默认 dihedral 30°, 参数))`，消除 `Toolbar.tsx:89,105` 与 `useTauriCommand.ts:41` 三处字面量 `30.0`；旧 `autoSegment` / `autoSegmentSmart` 钩子退役，统一走 `auto_segment_v2` IPC（后端 `b087c3c` 已落地，golden-sample 测试 74 passed）。
- 改动文件：`IntelligentSegmentPanel.tsx`（新增）、`Toolbar.tsx`、`useTauriCommand.ts`、`appStore.ts`、`i18n.ts`。

> 待先生 `cargo tauri dev` 手动复验：面板切算法 + 拖滑块后分区结果是否符合预期（headless 无法自动验证 Tauri 渲染）。

### Changed — 着色器分区高亮（方案 B，2026-08-08, `eea2fd1` + `485aa7d` + `f192f98`）
- **高亮改为 GPU 着色器方案 α**：删除 `SegmentHighlight`（重建几何叠加，是 giant 段卡死根因）与 `FillFaceHighlight`；主材质经 `onBeforeCompile` 注入 per-vertex `aSegLabel` + `uHighlightLabel`/`uHighlightColor` uniform，hover 切换从 O(F) 降到 O(1)，整模型 giant 段也能瞬时高亮
- **性能优化**：引入条件性 `frameloop="demand"`（`IdleFrameloop`：静止 1.5s 切 demand，任意交互唤醒），删除 `preserveDrawingBuffer`，`aSegLabel` varying 加 `flat` 限定符
- **高亮按工具门控**：仅 fill / picker / segment 工具计算并渲染分区高亮；brush / spray / view 等工具完全跳过，消除拖涂时每帧 React 整树重渲染的卡顿
- **hover 解粘滞**（迭代 35）：替换原「手动段强制粘性」守卫，改为真实区域一律接管高亮、仅极小碎面允许手动段保持粘性，并加陈旧标签立即清除 —— 修复「手动分区后填充工具高亮卡死十几秒」

### Fixed — STL 导入崩溃（2026-08-09, `69a17ea`）
- 修复打开球面等「同一纬度环共享轴坐标」的 STL（如 `Sphere.stl`）时直接闪退
- 根因：kiddo `KdTree` 在分裂轴上重合点超过默认 bucket（32）时 panic；修复按顶点/面 index 加 ~1e-4 确定性扰动（对几何与涂色半径无影响）
- 新增回归测试 `kdtree_coincident_axis_no_panic`（UV 球 50×320，单环 640 个重合轴面）；`cargo test --lib` 全绿 67 passed

### Added — v0.2 功能增强

#### 手动分区画笔 (Round 7-8)
- 新增 ✂️ 分区画笔工具（`PaintTool.Segment`），替代原 BFS 吸附方案
- 拖拽涂选面 → 松开鼠标创建新分区，支持多次拖拽创建多个分区
- Rust 端新增 `paint_segment_face` + `finalize_segment` 命令
- 前端 `currentSegLabelRef` + `segPaintedFacesRef` 跟踪拖拽状态并去重
- 手动标签起始偏移 100,000（`MANUAL_SEGMENT_OFFSET`），与自动分区隔离

#### GPU 拾取优化 (Round 6-7)
- 持久化 `pickGeo` / `pickMat` 引用，避免每次点击 clone 18MB Float32Array
- 共享 position/index buffer（`Float32BufferAttribute(sourcePos.array, 3)`）

#### 性能优化 (Round 6)
- 自动分割 Phase 3：法线一致性合并（normal-consistency merge），减少碎片区域
- MIN_REGION_CAP=30：小分区吸收阈值从 500 降至 30，避免过度合并
- 增量颜色更新：`updateFaceColors` 仅更新变更面，不重建整个几何体

#### UX 增强 (Round 3-5)
- **画笔光标**：3D 环形预览跟随鼠标，显示画笔半径和颜色
- **Space 平移**：按住 Space 切换为 Pan 模式，松开恢复 OrbitControls 旋转
- **分区边框**：分区视图中黄色线条高亮分区边界
- **喷漆差异化**：喷漆工具使用随机散布点区别于普通画笔
- **中英文 i18n**：全界面双语支持（`i18n.ts`），工具 tooltip 含当前参数
- **小分区合并**：Phase 4 将面数 < MIN_REGION_CAP 的碎片合并到最大邻居

### Changed
- 技术栈从 Python（PyQt6 + trimesh + pyvista）迁移到 **Tauri 2 + React + Three.js + Rust**
- 旧 Python 代码移至 `legacy/` 目录
- 自动分割算法从单阶段改为三阶段（二面角分裂 → 法线合并 → 小分区吸收）

### Fixed
- GPU 拾取 DPR 坐标偏移：使用 `gl.getPixelRatio()` 校正鼠标坐标
- OrbitControls 冲突：绘画时禁用旋转，松开恢复
- 分区标签冲突：手动标签使用 100,000+ 偏移，auto_segment 重跑不覆盖手动分区

---

## [0.1.0] — 2024-12 (初始版本)

### Added
- STL 导入（binary + ASCII，1.5M 面 4 秒加载）
- 自动网格分割（二面角阈值分裂）
- 基础画笔工具（brush_paint）
- 区域填充（fill_paint, fill_segment_paint）
- GPU 面拾取（颜色编码 ID → RenderTarget → 像素读取）
- 3D 视口（Three.js + @react-three/fiber + OrbitControls）
- 分区视图切换（segmentView toggle）
- 3MF 导出（quick-xml + zip）
- Zustand 状态管理（appStore）
- 进度条事件（segment-progress）
- 画笔设置面板（半径 + 强度）
- 颜色选择面板
- 状态栏（工具名 + 面数 + 分区数）

---

## 对抗式开发日志

本项目采用对抗式自循环开发（Adversarial Development Loop），每轮迭代经过：
`PLAN → REFUTE（对抗证伪）→ REVISE → IMPLEMENT → TEST → RETROSPECT`

### Round 7-8 — 手动分区画笔

**触发**：用户反馈"手动分区无用，光标不变，吸附无作用"

**对抗审查结论**（4 blocker）：
- 开放折线无法拓扑分割闭合曲面
- "两侧 BFS" 无种子面机制
- segment 模式点击即 flood-fill 与折线放置冲突
- 双击事件与 GPU pick 竞态

**修订方案**：推翻折线切割，改用"分区画笔"——拖拽涂面 + 松开 finalize

**改动文件**：`segment.rs`, `lib.rs`, `useTauriCommand.ts`, `Viewport.tsx`, `i18n.ts`

### Round 6 — 性能优化 + 法线合并

**触发**：用户反馈"太卡了" + 自动分割 1 个区域（cap=500 过度合并）

**对抗审查结论**：
- clone 18MB Float32Array 每次点击（GPU picker）
- computeVertexNormals 在每次 buildGeometry 中重算

**改动文件**：`Viewport.tsx`, `dihedral.rs`, `segment.rs`

### Round 5 — 小分区合并 + 进度条

**触发**：自动分割产生过多碎片区域

**改动文件**：`dihedral.rs`, `segment.rs`, `Viewport.tsx`

### Round 4 — i18n + 喷漆差异化

**触发**：界面缺少多语言支持，喷漆与普通画笔视觉无差异

**改动文件**：新增 `i18n.ts`, 修改 `Toolbar.tsx`, `Viewport.tsx`, 各组件

### Round 3 — 画笔光标 + Space 平移 + 分区边框

**触发**：初始 UX 评审，缺少基本交互反馈

**改动文件**：`Viewport.tsx`（BrushCursor, Space handler, SegmentOutline）
