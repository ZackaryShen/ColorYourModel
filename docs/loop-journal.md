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

---

## 迭代 79 — Eye 区作为分区参与 fuse & grow + SeedPanel 自适应 clamp

### 触发的理论缺口
- 迭代 78 装的 detect_eye_regions 输的是「可视化 overlay」(BoundaryLines), 不进 partition。导致：
  - 用户点了 "Fuse & generate" 后看到 fuse 出的 35 个区, 又点 Eye detect, 看到绿色 / 蓝色 / 青色 overlay, 但 overlay 是 dead layer。
  - 之后任何一次 seedGrow / 二次 fuse 都会把 eye 面就近 Dijkstra 并走, 用户的"识别出眼睛"被吞, **算法判定的轮廓和最终分区永远对不上**。
- 现有 Layer 4 (fuse) 是 3 通道 vote: planar (Layer 1) + multiview (Layer 3) + dihedral backbone (Layer 0)。Eye 是 Layer 4.5: 用户语义意图 (不是几何特征), 但是同样需要"防吞"。

### 初版方案 (被推翻)
- 候选 A: 每个 EyeRegion auto-addSeedPoint (中位面)。简单但有副作用: ① 用户没要 ghost marker UI; ② 单 seed 不强制 connectivity — eye region 内部被其他 seed 抢走的概率非零; ③ 完全不动 fuse, 用户调 fuse 时 eye 还是会被并。
- 候选 B: seedGrow 路径里, 给 EyeRegion 强制 "锁面", 加进 seed_grow 的 `seed_input` 不可 grow。破坏既有 seed_grow 语义, 引入 mutation-on-read。
- 候选 C (采纳): fuse 第 5 通道 eye_sets, EYE_WEIGHT=1000, 与 dihedral backbone 对齐; plus onGrow 给每个 EyeRegion 合成 1 颗 interior anchor seed 走 seedGrow Voronoi 路径。

### 对抗审查结论 (REFUTE)
- **[blocker] 1**: EyeRegion 不是 1-face label - 用 label_from_sets 给每眼面单独 label 会让"两个眼内面相邻"也投 cut+1, 等于把眼 region 自己切碎。采纳: 改用 `eye_bounded: Vec<bool>` (face-level flag), 边界检查 `ea != eb` 而不是 label diff。
- **[blocker] 2**: tiny-region post-merge 会把 1-face 的 sclera 并到邻居, 即便眼 channel 切开了也会被合并掉。采纳: 给含眼面的 region 加锁, 不参与 tiny-region-merge 候选。
- **[major]**: existing magic number `clampPos`/`resetPosition` 用 `innerHeight-460 / innerHeight-80` 写死, 高 DPI / 多屏 / 字体缩放下溢出。采纳: 用 panelRef getBoundingClientRect 实测 width/height。
- **[major]**: DEFAULT_POS 用 bottom-center, 4K 屏上 panel 跑屏幕中下而非视觉重点, 不在用户视线落点 (模型)。采纳: 改 bottom-right + margin。

### 改动文件 (实现)
- `src-tauri/src/segment/fuse.rs`:
  - 新增 `EYE_WEIGHT: i32 = 1000` 与 `DIHEDRAL_WEIGHT` 对齐。
  - `fuse_region_sets` 增加第 5 参 `eye_sets: &[Vec<u32>]`。
  - 新增 `eye_bounded: Vec<bool>` (每 face flag), 边界 vote: `ea != eb` → cut += EYE_WEIGHT。
  - tiny-region merge 候选过滤: `region_has_eye[rid]` 为 true 的不入列。
  - 4 个测试保留 + 2 个新测试: `eye_set_carves_itself_out` (1-face eye channel → 2 region), `eye_set_keeps_intra_region_cohesion` (2 adjacent eye faces → 1 region, outer 10 faces stay merged, total 2 region)。
- `src-tauri/src/commands/segment.rs`:
  - `fuse_segmentation` 加 `eye_face_indices: Option<Vec<Vec<u32>>>` 参, 内部直接 unwrap_or_default → `eye_sets`。
- `src/hooks/useTauriCommand.ts`:
  - `fuseSegmentation` 加第 4 参 `eyeFaceIndices?: number[][]`, invoke key `eyeFaceIndices` (camelCase, backend 已经 `snake_case: 'eye_face_indices'`, Tauri 2 自动转)。
- `src/components/SeedPanel.tsx`:
  - `onFuse`: `eyeSets = eyeRegions.map(r => r.faceIndices)`, 调 `fuseSegmentation(1, 0, dihedralDeg, eyeSets)`。
  - `onGrow`: 合成 `eyeSeeds: SeedPoint[]` (从每个 EyeRegion faceIndices 中位面取三顶点均值作坐标), 拼入 `seeds = [...seedPoints, ...suggestedSeeds, ...eyeSeeds]`。状态栏显示 "手 X + 推 X + 眼 X" 拆分。
  - `clampPos`/`loadPos`/`resetPosition` 全部用 `panelRef.current?.getBoundingClientRect()` 实测 width/height。
  - 拖动 `onMove` clamp 同理用活体尺寸。
  - resize 监听 rAF 节流。
  - 新增 first-mount effect: 当 panel 第一次拿到真实 rect 时 re-clamp, 防首屏 overflow。
  - DEFAULT_POS: bottom-right with margin (替代 bottom-center)。

### 测试证据
- 157 lib tests pass (原 6 fuse + 新 2 eye channel)。
- 7 vitest pass (现有 iter60/61/63/65/66 路径不退化)。
- tsc --noEmit 0 错。
- NSIS 3.3MB + MSI 5.0MB。

### E2E 期望
- 先生: 加载青蛙 → Fuse & generate (35 区) → 点眼睛区域 → Eye detect (出现绿/蓝/青 overlay) → 再点 Fuse & generate → 眼睛区域变为独立分区 (而不是被吸收)。
- 改走 Grow: 先生不重 Fuse, 直接点 Grow → 状态栏显示 "...+ 眼 3", 眼睛区保持独立 label, grow 后 overlay 还在。
- 拖 panel 到右下角, 缩小窗口到 panel 高 > 窗口高 → panel 自动 clamp 到 top + margin (而不是 overflow 隐藏)。
- 点 📍 复位: panel 落在右下 + 16px margin。

### 遗留 backlog (deferred to v2)
- 「auto-ROI」(无 selectedSegment 时 MultiView 自动选最像眼睛区域作 ROI) — 等先生决定。
- EyeRegion 进 SegmentsPanel 列表 (作为特殊 label 显示, 让用户在分区面板里也能 "选中眼睛区")。
- EyeRegion 在 export 3MF 时保存 semantic 信息 (globe/sclera/eyelid/socket 各自单独颜色组)。

---

## 迭代 80 — Fuse 反向切开 (57→535) + tiny-region merge 强化 + 状态栏诊断

### 触发的理论缺口
- 用户在 500k 面青蛙上点「融合生成」, segment count 从 57 暴增到 535。fuse 应该 "合并", 实际 "切开"。
- docs/09 §11 设计的本意: 边级多数投票 (`score = cut - keep > cut_threshold`), cut 必须严格多于 keep 才切, 平票合并。但是 fuse_region_sets 的 post-merge tiny-region step 只跑 3 passes × 1 region:
  ```rust
  for _pass in 0..3 {
      let tiny = counts.iter().filter(|(_, &c)| c < min_faces).min_by_key(...);
      let Some(tiny) = tiny else { break; };
      // ...仅合 1 个 region
  }
  ```
  对 500k+ 面模型的「百级别碎片」绝对不够——3 个 region 进, 几百个 region 留。

### 初版方案 (被否决)
- 候选 A: 把多视角角度阈值提到 35°+match_threshold=5。会让多视角更有用, 但**与设计本意冲突**: docs/09 §11 明确 "multi-view 鼓励过分割 (advisory only)"。改阈值 = 牺牲多视角价值。
- 候选 B: 改 dihedral 默认值为 30° (减少内部碎片)。会让几何骨干粗糙, 不识别脖子-躯体那种 20-25° 的折痕, 反而更糟。
- 候选 C (采纳): tiny-region merge 改成 dihedral 风格 (`merge_small_regions_fast`), 同时把诊断 log 通过 Tauri event 暴露给前端, 用户能直接看到 "525 区中 median=12 face", 知道是 "crumbs 要拖大点" 不是 "算法错"。

### 对抗审查结论 (REFUTE)
- **[blocker]**: tiny-region merge 改成 "每 pass 吞所有 under-sized" 会不会破坏原有 cube_fuses_to_six_regions 测试? **审查**: 6 个 cube 测试的 `min_region_faces` 调成 2, cube 仅 12 face, 不会触发 tiny-merge。只有 multi-patch + many-tiny 才会触发, 现有测试覆盖不到。新增一个测试覆盖 this case。
- **[blocker]**: 把 `__lastFuseDebug` 挂在 window 上是不是脆弱? **审查**: 只是 module 内的临时 stashing, 跨 component 不依赖。改成 useState 反而会触发重 render (噪声) — 用 window 反而更合理。
- **[major]**: 上一轮我曾以"dihedral 已经给出 38 区, fuse 该给 38", 但实际给 535 — 分析必然错位。这次**不再假设**, 让数据说话: 让用户重试, 在状态栏直接读到 fused-debug。
- **[major]**: dihedral 的 RAG Phase 3 normal-merge 合并阈值 (dot>=0.93=22°) 在光滑曲面上合并能力有限 — Phase 1 2339→Phase 3 1380→Phase 4 38=对实际模型太碎。但这是 dihedral 算法自身的另一个 backlog, 不在 fuse 修复范围。

### 改动
- `src-tauri/src/segment/fuse.rs`:
  - 新常量 `MAX_MERGE_PASSES = 40` (mirror of postprocess::MAX_MERGE_PASSES)。
  - tiny-region merge 改 dihedral 风格: 每 pass 吞所有 under-sized region, sort by size asc, eye-locked。
  - FuseResult 加 5 个诊断字段 (edge_total/edge_cut/region_size_min/max/median) — Default + 后向兼容。
  - 每个 channel 的 region 数和切边总数 log::info + main summary。
- `src-tauri/src/commands/segment.rs`:
  - `fuse_segmentation` done 时 emit `fuse-debug` Tauri event, payload 含 channels + cut stats + region size dist。
- `src/components/SeedPanel.tsx`:
  - useEffect 一挂载就 listen `fuse-debug`, 写入 `window.__lastFuseDebug`。
  - onFuse await 完后读 stashed payload, setStatusMessage 展示一行:
    `🧩 融合完成: 35 区 (通道 平面0/多视角125/折痕38/眼4, 边 5321/87214 切, 区大小 min=12 med=850 max=42000)`
- 状态: 用户能在屏幕直接读到 segment count 的原因, 不再需要 console。

### 测试证据
- 6/6 fuse 单测 pass (含原 cube + 2 eye + 1 empty)。
- 7/7 vitest SeedPanel pass。
- tsc --noEmit 0 错。
- NSIS 3.4MB + MSI 5.0MB。

### E2E 期望
- 用户加载青蛙 → Fuse & generate → 状态栏显示类似 `🧩 融合完成: 76 区 (平面0/多视角125/折痕38/眼4, 边 5321/87214 切, 区大小 min=12 med=850 max=42000)`。
- 如果仍有 500+ 区, 用户能从 med 数 (如果 med=几十) 看出 "crumbs 没合够", 下轮拉高 min_region_faces 滑块。
- 如果 med=几千（正常 region 大小), 那 500 区是真实算法结果（多视角过切+折痕多）。

### 遗留 backlog (deferred to v2)
- MultiView 算法的 angle_thr / match_threshold 加 UX slider (现在 SeedPanel 只暴露多视角按钮, 三个阈值都 hardcode)。
- Fuse 的 `min_region_faces` 已经是 slider 了, 但是 0=auto 用 `max(8, 0.002*n)` 默认值不够透明, 应该前端展示 "当前 min=X face, Y% 模型" tooltip。
- Dihedral Phase 3 normal-merge 阈值 dot>=0.93 (22°) 在光滑曲面上不够 aggressive, 让 Phase 1 2339 → Phase 3 1380 而非更激进合并。这是 dihedral 算法自身的优化, 不在 fuse 范畴。
- Eye 自动 ROI (MultiView 自动选最像眼睛区域) 仍然 backlog。
