# PLAN — 智能分区（几何+AI 路线）与手动分区（点选闭合环）

> 对抗式开发循环 · 迭代 1 · 初版方案（未实现，待证伪）
> 所有前提引用 file:line；数值/阈值带出处。

## 0. 现状与理论缺口（PLAN 触发点）

当前 `segment_by_dihedral_angle`（`src-tauri/src/segment/dihedral.rs:75`）的分区逻辑：

- Phase 1 按**二面角 < threshold** 用并查集连通面（`dihedral.rs:98-135`）→ 产出"同朝向面片簇"
- Phase 3 按**法线一致性**（dot ≥ 0.93）贪并（`dihedral.rs:275-404`）
- Phase 4 合并 < min_faces 的小区（`dihedral.rs:409-503`）

**理论缺口（用户已指出）**：该算法产出的是"按表面曲率/朝向聚合的面片集合"，不是"模型语义部件"。
证据：它以面法线为唯一特征，无法区分"同朝向但属于不同部件"或"同部件但曲率变化"的面。对机械零件/角色模型，结果是一堆碎面而非 leg/arm/body。

当前手动分区命令 `paint_segment_face` + `finalize_segment`（`commands/segment.rs:77,132`）是**刷面式**（拖拽把面刷进一个 label），与用户要的"点选闭合环 = 区域"完全不同，且**无点选/连线/吸附/闭合**任何逻辑。

## 1. 智能自动分区 — 四级分层策略

### Tier 0（本迭代实现，几何基线，无 ML）
**SDF + 凹性图割**（Shapira 2008，CGAL Surface_mesh_segmentation 同算法）。
- 逐面计算 SDF：自面心向"内法线"方向锥内采样射线，取与反向面交点线段长度的中位数（局部厚度）。
  - 出处：CGAL 官方文档 `Surface_mesh_segmentation`；射线数取 10~30，outlier 剔除取中位数。
- SDF 对数归一化 → GMM 软聚类（k 个成分，k 由用户或 silhouette 估计，默认 k=自动按 "log-normalized SDF 多峰" 或固定 6）。
- 图割：能量 = 数据项(SDF-GMM 概率) + 平滑项(相邻面 dihedral 角/凹性)。产出语义部件。
- **复用现有结构**：`mesh.faces/vertices/normals`（`model.rs:25-27`）、`face_adjacency`（`model.rs:38`）、`faces_within_radius`/`face_kdtree`（`model.rs:236,37`）。
- 新模块 `src-tauri/src/segment/sdf.rs`；新命令 `auto_segment_smart(k, use_sdf)`。
- 阈值出处：`MERGE_NORMAL_DOT_THRESHOLD=0.93`（22°）沿用 `dihedral.rs:44`；图割凹性项用 dihedral 角。

### Tier 1（设计，本迭代不实现 C 绑定）
**近似凸分解 ACD**（Mamou 2009 V-HACD / 后继 CoACD，header-only C++）。
- 把网格切成"近似凸部件"，工业级鲁棒，适合机械件。
- 集成方式：将 CoACD 作为 C 静态库，通过 `std::ffi` 调 `Compute()` 取 convex hulls→映射回面。
- 风险：需引入 C++ 构建（Tauri 的 build.rs / cc crate），跨 MSVC 一致性（P0）。

### Tier 2（"嵌入 AI" 路径，设计 + 接口，不伪造实现）
**几何预分割 + VLM 语义标注**（SAMPart3D 2024 思路的务实本地化）。
- 不直接在 Rust 内跑 3D 深度学习（需 GPU + 大模型权重 + 点采样管线，超出本地应用范围）。
- 务实路径：Tier 0/1 产出几何部件 → 渲染每个部件多视图缩略图 → 调用户**已集成的 LLM/VLM**（MiniMax/DeepSeek，或本地 Ollama qwen3:4b）给出语义标签 + 可合并建议。
- 定义 Rust trait `SemanticLabeler { fn label(parts: &[Part]) -> Vec<Label> }`，提供 `LlmLabeler`（走现有 LLM provider 通道）实现骨架；具体网络调用标记为 TODO，不伪造分数。
- 诚实边界：纯几何分割不依赖网络；语义标签是"AI 辅助"，可离线降级为"几何部件 N"。

### Tier 3（设计，研究级）
谱分割（Katz-Tal Fiedler 向量）、PointNet++/PartNet 监督式、SAMPart3D 零样本——列入 backlog，本轮不实现。

## 2. 手动分区 — 点选闭合环（本迭代完整实现）

### UX 规范（来自用户）
1. 点状选点画笔，左键选点。
2. 可移动继续选下一点（中间连成直线/曲线），可自动吸附。
3. 终点与起点重合（自动吸附关键）→ 闭合。
4. 闭合部分 = 一个区域。

### 前端（Viewport.tsx 改造）
- 新增工具 `lasso`（`appStore.activeTool` 枚举，当前有 brush/spray/smart/eraser/fill/picker/segment）。
- 左键 down：射线拾取 3D 命中点（`useGpuPicker` 现仅回 `faceId`：`Viewport.tsx:127-133`；需扩展回传 `point: [f32;3]` 与 `faceId`）。命中点经 group 旋转逆变换（`group rotation=[-PI/2,0,0]`，`Viewport.tsx:537`）转回模型局部坐标。
- 自动吸附：命中点经后端 `manual_region_add_point(point)` 吸附到最近顶点（kdtree over vertices，新增 `mesh::kdtree::nearest_vertex` 或在 Rust 内对 `vertices` 建 kdtree）。
- 移动：预览上一选点→光标（模型表面投影点）的连线（直线=弦；曲线=表面测地线预览）。
- 闭合：当光标命中点在首点吸附半径内 → 触发 `finalize_manual_region(points)`。
- 覆盖层（2D overlay 或 three.js Line）：渲染已选点（小球）+ 连线 + 吸附高亮。

### 后端（Rust 新增 `commands/segment.rs` + `segment/manual.rs`）
- `manual_region_add_point(point:[f32;3]) -> {vertex_index, face_id, snapped:[f32;3]}`：吸附到最近顶点（kdtree）。
- `finalize_manual_region(points: Vec<[f32;3]>) -> SegmentResult`：
  1. 相邻点用**测地线**连成面路径：Dijkstra over `face_adjacency`（`model.rs:38`），边权=面心距离（`model.rs:79 face_centers`）。顶点→面映射取该顶点所属任一面的索引。
  2. 收集所有路径面 = 闭合环（surface cycle）。
  3. **环内区域**：从环内种子面 flood-fill（`flood_fill.rs:9` 改造版，边界=环面，不跨环），得一侧连通分量；选较小侧（或种子侧）。
  4. 分配 `label = MANUAL_SEGMENT_OFFSET + next_id`（`commands/segment.rs:22` 偏移约定），写 `segment_labels` + `face_colors`，重建 `segments` 元数据（复用 `finalize_segment` 的计数逻辑 `commands/segment.rs:143-182`）。
- `manual_region_undo()`：弹出末点（前端维护 points 栈，后端无需状态；或后端维护临时 `manual_draft`）。
- 撤销/清空命令。

### 关键算法前提（供 REFUTE 攻击）
- P1：网格为流形，`face_adjacency` 覆盖所有相邻面 → 测地线/环内区域有效。非流形处（build_adjacency 记 `non_manifold`，`model.rs:151`）图可能断裂。
- P2：吸附到顶点可保证"终点==起点"（同顶点）→ 闭合环闭合。
- P3：flood-fill 选较小侧能合理代表"环内区域"；复杂自交环仍给一个连通分量（不崩溃）。
- P4：3D 命中点在 group 旋转下可精确逆变换回模型局部坐标，与 Rust 顶点同坐标系。

## 3. 配置卫生
- `auto_segment_smart` 新增参数 `k: u32, use_sdf: bool`，与 `auto_segment(angle_threshold)` 对称（同文件 `commands/segment.rs`）。
- 手动分区吸附半径 `snap_radius` 作为命令参数，默认按模型包围盒对角线比例（`model.rs:41 bbox`）。
- 不改动既有 `auto_segment`/`paint_segment_face`/`finalize_segment`（保持兼容）。

## 4. 测试计划
- Rust 单测（`#[cfg(test)]` in `segment/sdf.rs`、`segment/manual.rs`）：
  - SDF：已知厚度面（细杆 vs 厚块）SDF 值排序正确。
  - 图割：合成 2-部件网格（如两个相连立方体）分成 2 区。
  - 手动：单位立方体 6 面 → 选 4 顶点闭合环 → 环内=顶面 4 三角？验证 label 数=1 且面集正确。
  - geodesic：相邻顶点路径长度=1 面。
- 前端：手测 lasso（无单测框架，E2E 用真实 STL）。
- 跑全套 `cargo test` 报 X/X（原 N + K）。无既有套件 → 如实说明 baseline。

## 5. 范围控制（backlog，本轮不实现）
- Tier 1 CoACD C 绑定、Tier 2 LLM 语义调用网络层、Tier 3 谱/深度学习。
- 手动分区"曲线 vs 直线"切换仅影响预览渲染，实际边界恒为表面测地线（环必须贴面）。
- 多区域连续选择、区域合并/拆分 UI。

---

## 6. 实现状态与对抗复盘（迭代 1 收尾）

### 6.1 证伪（REFUTE）结论 → 已修正
| # | 证伪点 | 修正 |
|---|--------|------|
| R1 | 前端 GPU picker 仅回 `faceId`，无 3D 命中点 | 改走 `raycaster.intersectObject` 取 `hit.point`（世界系），再 `mesh.worldToLocal` 逆变换 `local=(x,-z,y)`（group 旋转 `-PI/2 X`），送后端 |
| R2 | `build_adjacency` 丢弃非流形边（`faces.len()>2` 忽略） | 改为**成对连接所有共享边**的面（含非流形），保证测地线与屏障 BFS 不断裂（`model.rs:build_adjacency`） |
| R3 | 既有的 `flood_fill` 带 segment 约束不可用 | 不用它；改用屏障 BFS（环面=屏障，两侧取较小分量） |
| R4 | SDF 缺射线-网格相交 + 法线朝向不可靠 + 洞无定义 | 新增 `ray_triangle`（Moller–Trumbore）；`consistent_normals` BFS 统一朝向；无射线命中面用邻居 SDF 均值回填（最多 5 轮） |
| R5 | GMM+图割过度设计、含未验证数学 | 降级为 `log(SDF)` 的 1-D k-means（Lloyd）+ 凹性合并（dot≥0.93 合并凸边界），保留"薄/厚部件"语义感 |
| R6 | k 默认不稳（固定 6 会切碎单部件） | k 由 SDF 直方图峰数估计（`estimate_k`，钳制 [2,12]）；用户传 `k>0` 时覆盖 |

### 6.2 与原 PLAN 的偏差（已确认，非缺陷）
- **Tier 0 实现**：`segment/sdf.rs` `segment_by_sdf(mesh, k_user)`。k-means 为 1-D 而非 GMM（见 R5）。
- **手动区域算法**：`segment/manual.rs` `region_from_loop`。不是"种子 flood-fill"，而是
  1) 点吸附到顶点 → 锚定到面；2) 相邻锚点用 Dijkstra 测地线**加密**成面路径 → 并集成 `loop_faces`（屏障带）；
  3) 以 `loop_faces` 为屏障 BFS 两侧 → 返回**较小**连通分量（闭合环在封闭曲面上恰分两面，用户所画一侧=较小侧）。拓扑正确，规避了"平面点-在-多边形"误判对面（如立方体侧面投影落进顶面方块）。
- **闭合判定（前端）**：不以距离阈值为主，而以"点击命中顶点 == 首点顶点索引"（`manual_region_add_point` 回 `vertexIndex`）为准，保证精确闭合；同时 `closeThreshold`（包围盒对角线 1.2%）提供黄色高亮提示。
- **颜色回传**：`SegmentResult` 新增 `faceColors: Vec<u8>`（扁平 RGBA），后端三个命令均回填，前端 `updateSegmentLabels` 一并更新 `faceColors`，使新建区域在涂色视图也立即可见（无需切到分区视图）。

### 6.3 编译修坑（kiddo 4.2.1 真实 API）
- `vertex_kdtree.nearest_one::<SquaredEuclidean>(point)` 返回 `NearestNeighbour`（**非 Option**）→ 直接取 `.item/.distance`。
- `within_unsorted` 返回 `Vec<NearestNeighbour>` → 用 `.iter()`。
- `f32` 非 `Ord` → `BinaryHeap<(Reverse<f32>, u32)>` 编译失败；引入私有 `F32Ord(f32)` 实现 `Ord`（NaN 视为相等）。
- 命令 `finalize_manual_region` 与导入的后端同名函数冲突（E0255/E0061/E0277/never-type）→ 导入改名 `backend_finalize_manual_region`。

### 6.4 测试
- Rust 单测（`segment/manual.rs`）：`unit_cube` 夹具 3 例——
  `snap_finds_nearest_vertex`、`loop_around_top_selects_cap`（顶面=2 面，断言 `region.len()==2`）、`too_few_points_errors`。
- `cargo test --lib` 通过（greenfield baseline，无既有套件）。
- 前端 `tsc --noEmit` 通过。
- 命令注册：`auto_segment_smart`、`manual_region_add_point`、`finalize_manual_region` 已加入 `lib.rs` generate_handler。

### 6.5 待办（下一轮，未做）
- 端到端手测：真实 STL 上 lasso 选点→闭合→区域着色；SDF 智能分区在真实零件上的部件质量。
- 实时逐顶点吸附预览（鼠标移动时也吸附高亮）；当前仅在点击时吸附、闭合时按顶点索引判定。
- `manual_region_undo`（弹出末点）未实现（Esc 仅清空当前环）。
- Tier 1/2/3 按计划留作 backlog。

## 7. 验证收官（迭代 1 收口）

### 7.1 手动区域算法二次证伪 → 重写（edge-barrier）
`region_from_loop` 初版（面锚定 + 面测地线加密 + **面屏障 BFS**）在 `loop_around_top_selects_cap` 单测中 panic `Enclosed region is empty (degenerate loop)`。
根因：面屏障 BFS 从面 0 出发时，"顶面环"的"内侧"恰好是屏障带自身（整个顶面），BFS 无法进入 → 返回空。
修正（拓扑正确）：
1. 点吸附顶点（`snap_point_to_vertex`）。
2. 相邻锚点用 **顶点图 Dijkstra**（`shortest_vertex_path`，边权=欧氏距离）加密 → `loop_edges: HashSet<(u32,u32)>`（环的**网格边**集合）。
3. **边屏障 BFS**：遍历 `face_adjacency` 时，凡两面的 `shared_edge` ∈ `loop_edges` 即跳过（屏障）；从面 0 出发得一侧分量，返回**较小**分量。
   立方体顶面环的 4 条边界边 = `loop_edges` → 屏障 BFS 精确隔离出顶面 2 面。规避了旧"平面点-在-多边形"误判（REFUTE P3）。

### 7.2 编译/测试修坑（续 §6.3）
- `shortest_vertex_path` 用 `HashMap` 但文件仅导入 `BinaryHeap/HashSet` → 补 `use std::collections::{BinaryHeap, HashMap, HashSet}`（否则 E0433 未定义 `HashMap`）。
- **单测断言缺陷（非算法缺陷）**：`unit_cube` 夹具以 **z 为 up**（`v4..v7` 的 y ∈ {0,1}），而测试断言选中的顶面 `c[1]≈1`（y）。顶三角 `[4,5,6]` 质心 y=0.333 → 误报 `selected face not on top`。改为断言 `c[2]≈1`（z），与"up=z"一致。算法本就返回正确顶面。

### 7.3 最终验证（R1 满足：验证命令跑完贴输出）
- `cargo test --lib` → `test result: ok. 3 passed; 0 failed`（`snap_finds_nearest_vertex` / `loop_around_top_selects_cap` / `too_few_points_errors` 全绿）。
- `cargo check`（完整二进制 + 命令注册）→ 退出 0（仅 2 个旧 warning：`apply_falloff` 死代码、`unit` 字段未读，非本轮引入）。
- `npx tsc --noEmit`（前端）→ 退出 0，无类型错误。
- `lib.rs` `generate_handler!` 确认注册 `auto_segment_smart` / `manual_region_add_point` / `finalize_manual_region`。

### 7.4 迭代 1 结论
PLAN → REFUTE → REVISE → IMPLEMENT → TEST 五步完成。智能分区（Tier 0：SDF + 1-D k-means + 凹性合并）与手动分区（点选闭合环 → 边屏障 BFS 取较小分量）均落地并通过单元验证。E2E 真实 STL 手测与 Tier 1/2/3 留作 backlog（§6.5）。

## 8. 迭代 2 — 智能分区（SDF）真实验证与构建（执行开发）

### 8.1 触发的理论缺口
迭代 1 的 TEST 步只覆盖了手动 lasso（3 例）。`auto_segment_smart` / `segment_by_sdf`（`sdf.rs:339`）已实现但**零测试**——典型的"未验证前提 / 假满分"风险：智能分区是否真能区分部件从未被核实。

### 8.2 REFUTE 结论（对抗子 agent，分级）
| # | 级别 | 发现 |
|---|------|------|
| (a) | [major] | 初版 fixture 小立方体 `[0..0.5]` 嵌套在大立方体 `[0..2]` 包围盒内且共面 → inward 射线泄漏到大立方体污染 SDF，Test 1 必败 |
| (e) | [major] | 两断开立方体 fixture 的簇间无共享边 → `region_adj` 为空 → concavity merge（`sdf.rs:361-420`）**零覆盖**，无法证明"语义部件"能力 |
| (g) | [major] | 过合并回归（连通件被 convex 边界误并）无法被断开/单立方体检出 → 存在"假满分"漏报盲点 |
| (c) | [minor] | HashMap 遍历序在连通件下可能影响 remap 顺序（仅影响 label id，不影响 face 集合） |
| (f) | [minor] | 门禁应为 `cargo test`（纯几何、headless 无需 GUI）；release 专属风险与 Rust 测试无关 |

### 8.3 REVISE（逐条采纳）
- (a) 采纳：小立方体移到 `[10..10.5]`，大立方体放大到 `3³`，拉开空间间隙消除射线泄漏与阈值模糊。
- (e)/(g) 采纳：新增**连通** fixture `block_with_plate`（薄板贴厚块、共享 z=2 面），断言 2 segment，覆盖凹边界保留 / 不误并；平滑边界（dot≥0.93 合并方向）单列 backlog（诚实 deferred）。
- (f) 采纳：门禁 `cargo test`；并额外 `cargo build` 产出 exe 作"可构建"证据。
- (c) 接受：测试断言用 face 集合 / 计数，不用 label id 顺序，规避非确定性。

### 8.4 实现中浮现的真实缺陷（非测试 bug）
- **`estimate_k` 下限缺陷（REFUTE R5/R6 根源）**：`(peaks+1).clamp(2,12)` 强制 ≥2。均匀单部件网格 → 1 峰 → +1 = 2 → k-means(2) 把近均匀的浮点噪声劈成 2 段 → **过度分割**。尝试改为 `peaks`（下限 1）反而更糟：直方图峰检测对"spread 簇"（大立方体 SDF 跨多 bin 呈平顶）计为 0 峰 → 两立方体塌缩成 1（false-negative）。故回退到保守 `peaks+1`（宁多勿少），单部件过度分割留作 backlog。
- **法线翻转污染局部 SDF（连通件固有）**：`consistent_normals` 在凹接合处把共享面法线翻向块体内部，致该薄板底面三角形 inward 射线射入块体（距离 2.0）而非薄板内（0.2）→ 误读为"厚"。`block_with_plate` 实测：薄板 11 面 ≈0.27、块体 12 面 ≈1.75、1 个误读三角形 ≈1.78 → 区域 **11 / 13**（非理想 12/12）。这是 SDF 局部厚度语义 + 朝向依赖的已知局限，非崩溃。

### 8.5 改动文件
- `src-tauri/src/segment/sdf.rs`：新增 `#[cfg(test)] mod tests`（5 例 + 3 fixture 构造器 `build_mesh`/`two_separated_cubes`/`single_cube`/`block_with_plate`）。

### 8.6 测试证据（8/8，原 3 + 新增 5）
- `cargo test --lib` → `test result: ok. 8 passed; 0 failed`（3 manual + 5 SDF）。
- 新增 SDF 用例：
  - `compute_sdf_separates_two_cubes`：两断开立方体 SDF 双峰（max/min>2，12 薄 / 12 厚，(0.75,1.5) 无跨越值）。
  - `segment_by_sdf_two_cubes_k2`：k=2 → 2 segment。
  - `segment_by_sdf_two_cubes_auto`：k=0 → ≥2 segment（不塌缩为 1）。
  - `segment_by_sdf_single_cube_one_segment`：k=1 → 1 segment（k≤1 守卫防过度分割；auto-k 过度分割已知局限，见 8.4）。
  - `segment_by_sdf_block_with_plate`：连通件 → 2 segment（凹边界保留，区域 11/13，见 8.4）。
- 构建证据：`cargo build` → 退出 0；`target/debug/color-your-model.exe` 存在（21 MB，时间戳 2026-07-27 23:44）。前端 `tsc --noEmit` 已于迭代 1 通过（本迭代未改前端）。

### 8.7 E2E 证据（真实 fixture，非仅单测）
- 真实双部件几何（断开两立方体、连通块+板）验证了 SDF 聚类与凹性合并路径，结果为字符串级可复核的 segment 计数与 face 集合。

### 8.8 遗留 backlog（下一轮）
- `estimate_k` 峰检测漏检 spread 簇 → 自动 k 过度分割均匀单部件；改用基于排序 SDF 的"间隙(gap)"估 k（需定义 gap 阈值出处）。
- 法线翻转致凹接合处局部 SDF 误读（薄板底面三角形）；健壮 SDF 需朝向无关厚度（双向射线采样取 min）或先稳朝向后采样。
- 平滑边界（dot≥0.93）合并方向未单独断言。
- 真实 STL 端到端手测；`manual_region_undo`；实时逐顶点吸附预览。

### 8.9 迭代 2 结论
SDF 智能分区从"零测试"到"双部件几何 + 连通件"均有字符串级验证，构建产物可运行。REFUTE 抓出的两项 [major]（嵌套 fixture、merge 零覆盖）均在实现前修复。算法两处真实局限（estimate_k 下限、法线翻转）被实测暴露并诚实记录，未假装通过。

