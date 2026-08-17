# Loop Journal — Adversarial Development Loop 落盘

> 按 `adversarial-development-loop` 第⑥步 RETROSPECT+JOURNAL 当场落盘。
> 每轮一个条目，模板见该 skill 的 Loop Journal 条目模板。
> 当前项目主线分支：`feat_theme_persist_paint_ux`。

---

## 迭代 N — 眼睛区域语义识别（Globe / Sclera / Eyelid / Socket）

### 触发的理论缺口
- 现有三层算法（Layer1 planar / Layer3 MultiView / Layer2 cross-section）+ Layer4 边级多数投票融合，对**眼睛内部细粒度语义**无能为力：
  - planar 根本不进高曲率眼球曲面；MultiView 按法向 20° 生长，弯曲眼球必切碎、平坦额头跨眼部合并；cross-section 是平面证据非面级。
- 用户要在 cat.stl 类单色模型上拆出 眼球/眼白/眼睑/眼眶 4 个语义区，这是现有检测器覆盖不到的子任务 → 触发「形态-语义」新层。

### 初版方案（被推翻点）
- 候选 A：MultiView 给「眼周 ROI」→ ROI 内局部球拟合 + concavity 投票细分。
- 球度 `S = 1/(1+e/r²)` 区分球 vs 平面；眼睛/眼白用「到 ROI 边缘距离」判据；concavity>0.5 判眼眶。
- 完整 PLAN 见 `docs/10-眼睛区域语义识别.md` §1–§9。

### 对抗审查结论（REFUTE，独立 agent，[blocker]/[major]）
- **[blocker] 1**：球度 S 对**共面退化**——平面 RANSAC 球半径→∞、残差→0，S≈1，平坦脸颊与球面眼球 S 均≈1，主判别器作废。
- **[blocker] 2**：距离判据写反——"到 ROI 边缘 < 0.6·r_roi" 应为「外缘=眼白」，方案写反成「小距离=眼球」。
- **[blocker] 3**：**MultiView ROI 前提虚假**——MultiView 按法向一致(20°)生长，弯曲眼球法向跨 >20° 必切碎、平坦额头跨眼部合并；实际产物是「额头+眉毛+眼睑+鼻子」一坨，不产生「眼周 ROI」。→ 整条候选 A 链路不可达。
- **[major]**：`face_concavity_scores` 返回离散 {0,.333,.667,1.0}，`>0.5` 等价「≥2/3 顶点凹」，与既有 concavity 语义错位（应直接用 `vertex_concavity` 的 0/1 flag）。
- **[major]**：闭眼退化假设错误——折痕 S>0.6 会产**假 globe**。
- **[major]**：`compactness_threshold` 死参数从未使用。
- **[major]**：候选 B（全 mesh 形态聚类）否决理由「内存爆」虚假（1.5M 面 × 3 float ≈ 17MB）。
- **[major]**：v1 四区与「单色不可靠」自相矛盾。
- **[minor]**：`EyeRegion → suggestedSeeds` 接线未定义。

### 修订方案（逐条 采纳/反驳/backlog）
- 采纳 #1：球度改用**球-平面 inlier 计数比** `R = sphere_inliers/(sphere_inliers+plane_inliers)`，face 同时拟合球与平面，取 inlier 数 → **彻底消除平面退化**。
- 采纳 #2：距离判据改为三层环带——ROI 边缘 < 0.4·r_roi = 眼白(外缘)、0.4~0.85 = 眼球(中环)、>0.85 = socket 内壁；单色模型简化为环带。
- 采纳 #3：**v1 入口 = 仅 lasso**（先生手动圈选眼部），MultiView 自动 ROI 退 v2（前提证伪）。
- 采纳 #4：concavity 用 `vertex_concavity` 0/1 flag 做面投票（≥2/3 顶点凹 → 凹面）；折痕 vs 眼眶靠二面角峰度区分。
- 采纳 #5：闭眼 fallback——ROI 内凸球冠 < 30 face → globe/sclera 标 absent，状态栏提示「未发现眼球，疑似闭眼」。
- 采纳 #6：删 `compactness_threshold`。
- 反驳 #7：候选 B 否决理由改为「全 mesh 形态聚类无簇锚点，跨模型不可靠」（撤回「内存爆」）。
- 采纳 #8：v1 sclera 标 **Heuristic label**，confidence ≤ 0.5（单色本质限制）。
- 采纳 #10：`EyeRegion → (point, face_index) → SeedSuggestion` adapter，先生点 ghost marker 采纳。
- REVISE 全文见 `docs/10` §12。

### 改动文件（实现阶段）
- 新增 `src-tauri/src/segment/eye/mod.rs`：`detect_eye_regions(mesh, roi_faces) -> Vec<EyeRegion>`，球-平面 inlier 比 RANSAC + 距离分环 + 闭眼 fallback。
- 复用：`segment::concavity::vertex_concavity`、`segment::postprocess::crease_strength_deg`、`mesh.face_adjacency`/`face_kdtree`/`face_center`/`vertices`/`faces`/`compute_normals`。
- 新增命令 `detect_eye_regions(roi_faces: Vec<u32>)`（lib.rs invoke_handler 注册）。
- 前端：`types/mesh.ts` EyeRegion；`useTauriCommand.detectEyeRegions`；`SeedPanel`「👁 眼睛识别」按钮（取当前选中 segment 的 faces 作 ROI）；`i18n` 文案；`Viewport` 用 amber/pink/indigo 渲染三类边界。

### 测试证据（实现后补 X/X，原 N + K）
- 后端单测：lasso 球面→Globe+Sclera；lasso 凹半球→Socket；闭眼假 mesh→Eyelid only + 状态提示；cube ROI 空→[]。
- 前端 tsc 干净 + vitest（含按钮/状态路径回归）。

### E2E 证据
- 先生装包 → 圈 cat 眼睛（lasso）→ 点「👁 眼睛识别」→ 看标签（琥珀=眼球、粉=眼睑、靛=眼眶；眼白为 Heuristic 置信度低）。

### 遗留 backlog（v2）
- MultiView 自动 ROI（前提未验证，需先实测 cat.stl 上 MultiView 产物规模）。
- EyeRegion 入 Layer 4 fuse（边级投票加第三个投票者）。
- 暴露参数面板（ball_radius_ratio / sphere_inlier_threshold / min_region_faces）。
- 端到端 PSB/真实模型基准。
- 眼白真实辅助判定（依赖 texture/UV，v1 纯几何 Heuristic）。
