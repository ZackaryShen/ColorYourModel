# 设计：视口内 Ctrl+点击 多选分区合并（Issue #16）

日期：2026-10-19 · 分支：bugfix_012 · 状态：已批准（用户在简报中从 A/B/C 方案中拍板 B）

## 背景

Issue #16：自动播种猜错边界时，用户只能重新播种。诉求 = 直接在模型上多选已分区区域并合并。
面板内合并已存在（`SegmentsPanel` 本地 `picked` 状态 + `merge_segments` 后端命令），缺的是模型表面的拾取交互与可见性。

## 目标

在 3D 视口中 Ctrl+点击（⌘+点击）模型表面，把点击面所属分区加入/移出合并多选集；≥2 个时出现浮条，一键合并（复用 `merge_segments`）。面板行多选与视口多选共享同一状态。

## 非目标

- 播种阶段（seed_grow 之前）的合并语义。
- 面板合并 UI 重构、GitHub issue 回复、视频教程（另行推进）。
- Ctrl 悬停时整区高亮预览。

## 交互设计

1. **拾取**：`Viewport` `onPointerDown`（`Viewport.tsx:1668`）在 gizmo 与 Alt 守卫之后、**`isViewTool` 早退之前**、各工具分支之前插入：`e.ctrlKey || e.metaKey` 且 `meshData.segmentLabels` 存在且命中模型 → 取 `segmentLabels[hit.faceIndex]`，调 `toggleMergePicked(lbl)`；未命中 → 忽略（保持现状，让轨道旋转）。（修正记录：最初写在 view 守卫之后，但 View 是默认工具，首跑用户不切工具就 Ctrl+点击，拾取会完全失效——桌面实测发现后上移。）不占用：Ctrl 目前仅用于 Ctrl+滚轮调笔刷（`Viewport.tsx:2222`），无 pointerdown 冲突。Alt 旋转优先级高于拾取；View 工具下 Ctrl+点击拾取、普通拖拽仍是旋转。
2. **反馈**：
   - 状态栏提示首次拾取成功：新 i18n key `merge.hint`（zh：Ctrl+点击已分区模型可多选区域合并；Esc 清空 / EN 同义）。
   - `SegmentOutline`（`Viewport.tsx:514`）由单 `selectedSegment` 扩展为 `labels: number[]`：锚点（=selectedSegment，即 picked[0]）沿用现有样式，非锚点 picked 用辅助色描边。props 改名 `labels`，memo deps 同步。
3. **浮条**：新组件 `Viewport/MergeBar.tsx`（HTML overlay，画布底部居中）：`mergePicked.length >= 2` 时显示 `t("segments.mergeInto", n, anchorName)` + 合并按钮 + ×（`clearMergePicked`，tooltip `merge.clearTitle`）。合并逻辑走共享 hook。
4. **共享合并逻辑**：新 `src/hooks/useSegmentMerge.ts`：`const { mergePicked, merging } = useSegmentMerge()` → 内部取 store `mergePicked`，调 `useTauriCommand().mergeSegments(picked[0], picked.slice(1))`；成功后 `setStatusMessage(t("segments.merged", n, moved))`、`setMergePicked([anchor])`、`setSelectedSegment(anchor)`。`SegmentsPanel.runMerge` 改为调用该 hook（删除本地 `picked` state，行点击改写 store）。
5. **清除**：Esc 清空（挂进现有窗口 keydown，`mergePicked.length > 0` 时拦截）；浮条 ×；面板行 Ctrl+点击取消单个。

## 状态（appStore）

- 新增 `mergePicked: number[]`（有序，首元素为锚点/保留区）；`toggleMergePicked(label: number)`（追加去重/移除，保持顺序）；`clearMergePicked()`；`setMergePicked(list)`。
- `toggleMergePicked` 顺带把 `selectedSegment` 设为新锚点（picked[0]），维持现有高亮语义；清空时不动 `selectedSegment`。
- 重置路径清空：初始 state、`setMeshData` 换模型清理块（`appStore.ts:553` 一带）、面板"重置分区"成功后（`onResetSegmentation`）。
- 过期清理：`SegmentsPanel` 现有 `segments` 变化监听（`SegmentsPanel.tsx:102`）改为 `setMergePicked(pruned)`；`MergeBar` 内同规则一行 effect（共享 `usePruneMergePicked` 不强求）。

## 数据流

click → faceIndex → `segmentLabels[faceIndex]` → store.mergePicked → ①SegmentOutline 描边 ②MergeBar 显隐 ③面板行 chip（`itemPicked` 样式沿用）→ 合并按钮 → `merge_segments` → 后端返回新 SegmentResult → store.segments/segmentLabels 更新 → prune effect 收敛。

## 错误处理

- 点击面 label 不在 `segmentIds`（未分区/脏数据）：忽略并提示 `merge.hint`（引导先分区）。
- `mergeSegments` reject：走 `useTauriCommand` 现有错误状态提示，`mergePicked` 原样保留。
- 撤销/重跑导致 label 失效：prune effect 兜底；后端存在性校验兜底。

## 测试

1. store（`appStore.merge.test.ts`）：toggle 追加顺序/去重/移除、清空路径、`selectedSegment` 锚点联动。
2. `SegmentsPanel.test.tsx`（新增）：mock `useTauriCommand`，行 Ctrl+点击 → store 多选 → 合并按钮以（锚点, 其余）顺序调用命令；成功后 picked 收敛为 [锚点]。
3. `useSegmentMerge` 经由 2 覆盖，不单测。
4. Viewport 指针路径不做自动化（与现有 seed/lasso 同待遇），给手动验证步骤。

## 手动验证

导入多分区模型（samples/ 有现成）→ Ctrl+点击 2–3 个区域 → 描边 + 浮条出现 → 合并 → 面板数量减少、锚点保留 → Esc/× 清空 → View 工具旋转不受影响 → 画笔工具 Ctrl+滚轮调径不受影响。

## 风险

- OrbitControls 对 Ctrl+Left 的默认映射（pan）：ControlsBridge 已重映射鼠标键（iteration 14），Ctrl+click 无拖动应无位移；若有，实测定点。
- 大 mesh 上多标签描边 memo 开销：仅 picked 集合参与，量级同现状。
