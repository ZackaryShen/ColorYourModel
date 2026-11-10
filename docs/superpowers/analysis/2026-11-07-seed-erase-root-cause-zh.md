# 种子橡皮擦：根因分析（取证结论）

> 日期：2026-11-07 · 分支 `feature_012_governance` · HEAD `64d544e`（被测代码为 `df094b7`）
> 取证应用：`tools/seed-erase-forensics/`（浏览器可跑，含自测）
> 完整英文证据链：`docs/superpowers/analysis/2026-11-07-seed-erase-coordinate-space.md`
> 独立复算：`tools/seed-erase-forensics-verify/`（与取证应用互不通气地各写一份，数字一致）
>
> **状态（2026-11-10，round 9）：§5 的修复已落地** —— `src/components/Viewport/Viewport.tsx`
> （屏幕空间匹配 / smear 修复 / auto 守卫顺序 / billboard）+ 新增 `src/utils/seedErase.ts`
> （14 个单测）。`tsc` 零错误、`vitest 161/161`。验收记录见根目录 CHANGELOG。

---

## 0. 一句话结论

**种子存的是「模型局部坐标」（STL Z-up），而 `Vector3.project()` / `Raycaster` 工作在「世界坐标」，两者之间差一个场景组旋转 `Rx(−π/2)`（`Viewport.tsx:2580`）。** R6 / R7 / R8-first
三轮修复全部是在**跨坐标系比较**——所以无论阈值调多大都全 miss。HEAD（R8-final）的**点击**路径终于把两边都放在局部空间里，空间是对的；但**拖拽擦除路径仍把世界坐标喂给局部比较器**（永远 miss），且点击容差只剩 **≈3px**。这就是"一直没解决"的两层原因。

---

## 1. 坐标系事实（不是猜测，逐条可查）

| 事实 | 证据 |
|---|---|
| 模型 + 种子标记都在 `<group rotation={[-π/2,0,0]}>` 里 | `src/components/Viewport/Viewport.tsx:2580`（网格 2581、标记 2674-2675） |
| 组上没有 position/scale，唯一变换就是这一个旋转 | 同上；`CameraFit` 注释 `Viewport.tsx:175-176` |
| 种子 = 点击三角形的**质心**，取自 `meshData.vertices`（原始 STL 顶点，Z-up） | `Viewport.tsx:2063-2078`；类型文档 `src/types/mesh.ts:42-49` |
| 后端所有 seed 接口同样吃/吐 model-local | Rust `recommend.rs:41-49`、`seeded.rs:76-81`、`manual.rs:63-85` |

所以：

```
world = R · local,  R = Rx(−π/2) = [[1,0,0],[0,0,1],[0,−1,0]]
local (x,y,z)  →  world (x, z, −y)
world (X,Y,Z)  →  local (X, −Z, Y)
```

**跨空间代价**：把一个局部点当世界点用，误差 = `√2 · ρ_yz`（到局部 X 轴的距离），最大到模型尺度的 `1/√3`。
它按种子固定（与点击位置无关）——这正是 R7 观测到的"235–652px、且不随点击位置线性变化"的签名。

---

## 2. 三轮失败逐轮对齐（数字都能复现）

| 轮次 | 当时的写法 | 真实误差 | 复现结果 |
|---|---|---|---|
| **R6/R7** | `project(种子局部坐标)` 后和点击像素比 | `zoom · √2·ρ_yz`，模型量级 | 独立复算：中位 **549px**、最大 **721px**，**71.4%** 的表面落在实测 235–652px 区间内；误差只随 (y,z) 变、和屏幕位置无关。R6 另叠了一个 **~59px** 的"窗口绝对坐标 vs 画布相对坐标"偏移（`73ec499` 修掉） |
| **R8-first** | `raycaster.ray.distanceToPoint(种子局部坐标)` | 世界射线 vs 局部点，等于把像素当世界单位 | 549px ↔ zoom **6.91**；13293px ↔ zoom **167.22**（≈ 拟合后放大 24 倍——为了点中 2px 小点，人一定会放大），两处都 **0.00% 残差**复现。"射线数据是垃圾"是错的：射线没问题，喂进去的点在另一个空间 |
| **R8-final（HEAD）** | 点击：`worldToLocal(hit)` vs 局部种子 ✅ | 空间对了 | 点击居中命中 = 0.000–0.067wu ✅ |
| **R8-final 拖拽** | `eraseNearWorldPoint(raycastFace().point)` ← **世界点** | `√2·ρ_yz` = 10.7–25.9wu，阈值 0.24wu → **超 61–203 倍** | 100% miss ❌ |

> 手稿 §3.4 把教训写成"three.js 相机 API 在此正交相机场景不可信，别再试屏幕方案"——**这个教训是错的**，会误导下一轮。`setFromCamera` 恰恰是播种/涂色/悬停一直好用的那条路径；`project()` 从来没有坏过，坏的是输入空间。

---

## 3. HEAD 仍然"点了没反应"的四层原因

1. **拖拽擦除 100% 失效**（`Viewport.tsx:2125-2126`）：世界命中点 vs 局部种子。
   更隐蔽的是：**点一下擦掉成功后，只要指针移动 ≥1px 就进这条路径**，状态栏立刻被 `erase miss ... wu` 覆盖——**擦成功也会看起来像失败**。
2. **点击容差 ≈ 2.5px**：阈值 `seedMarkerSize × 2.5` 在实测案例里 = **2.52px**；斜掠表面还要再乘 `cos`（实测 85° 时只剩 0.21 倍球半径）。用户瞄准的是一个 2px 的目标。取证应用实测：**点击居中 PASS，偏移 4px 立即 FAIL**。
3. **自动模式守卫挡在擦除分支前面**（`Viewport.tsx:1874`）：切到「融合生成」后红色 marker 仍然显示、点击却什么都不做（`setSegmentMode` 不清 `seedEraseMode`）。
4. **billboard 错 90°**（`Viewport.tsx:2789`）：把相机**世界**四元数塞进旋转组的**子**节点，stem/叶子被转进模型内部、被深度测试遮住——视觉上"红点"只剩球体（≈2.5px），更难瞄。位置不受影响（不是位置 bug）。

次要：Ctrl+点击会被静默吞掉；eye-region 的种子从不进 `seedPoints`，天然擦不掉。

---

## 4. 取证应用：怎么看到以上每一条

```bash
cd tools/seed-erase-forensics
python3 -m http.server 8137 --bind 127.0.0.1   # 打开 http://127.0.0.1:8137/index.html
node probe.mjs                                 # 无头版，同一套数字
```

应用把 `Viewport.tsx` 的几何/相机/标记/阈值逐行搬了过来（同一套 `–π/2` 组、同一套 CameraFit 公式、同一个 sapling 模型、同一个 billboard bug），一次点击同时评估 6 种匹配策略：

| 策略 | 对应代码 | 结论 |
|---|---|---|
| S1 `project(局部种子)` | R6/R7 | **FAIL**（差 228–335px） |
| S2 `project(localToWorld(种子))` | 修正版 | **PASS**（<1px） |
| S3 `ray.distanceToPoint(局部种子)` | R8-first | **FAIL** |
| S4 `ray.distanceToPoint(世界种子)` | 修正版 | **PASS** |
| S5 `worldToLocal(hit) vs 局部种子` | **HEAD 点击路径** | 居中 PASS / **偏 4px FAIL**（容差 2.5×size ≈ 3px） |
| S6 `hit(world) vs 局部种子` | **HEAD 拖拽路径** | **FAIL**（差 10–26wu） |

自测（Auto self-test）：**12/12 通过**——两个场景（居中 / 偏 4px × 六种策略）都按预期 FAIL/PASS。
应用还带一个 **"erase = S2·fixed"** 开关：切过去之后，点击红色种子就能稳定擦除——修复方案的现场证明。
截图：`tools/seed-erase-forensics/screenshot-selftest.png`。

---

## 5. 修复方案（4 处小改动，已由取证应用验证）

1. **擦除统一走屏幕空间**（click 和 smear 共用同一个函数）：
   ```ts
   // 局部种子 → 世界 → NDC → CSS 像素；像素半径按 marker 视觉大小给
   const v = new THREE.Vector3(s.x, s.y, s.z);
   meshRef.current.localToWorld(v);          // ← 三轮修复都漏掉的这一步
   v.project(camera);
   const sx = (v.x * 0.5 + 0.5) * rect.width;
   const sy = (-v.y * 0.5 + 0.5) * rect.height;
   // 阈值：markerSize × 3.5 的世界大小换成像素（px/wu = ortho.zoom），再夹到 [14, 48]px
   const pxRadius = Math.min(48, Math.max(14, seedMarkerSize * 3.5 * ortho.zoom));
   ```
   smear 直接用指针像素（不再 raycast），**指针移到空白处也能擦**；点击不再要求命中模型表面。
2. **删掉死代码** `eraseAtScreen`（`Viewport.tsx:1931-1990`，定义了从未调用，且带着 R8-first 的错误假设）。
3. **修 billboard**：`q = parent.getWorldQuaternion().invert() * camera.quaternion`，让字形真正朝向相机（视觉/可瞄性）。
4. **把 erase 分支提到 auto 守卫之前**（或在 `setSegmentMode("auto")` 时清掉 `seedEraseMode`）。

改完 `npx tsc --noEmit` + `npx vitest run` 应保持全绿（当前检出没有 `node_modules`，需要先装）。

---

## 6. 还需要真机确认的两件事

1. **HEAD 是否被真正实机验收过**：手稿写"待验收"，而代码层面点击路径空间是对的——如果实机仍全 miss，最可能是 §3 的第 2/3/4 条。
2. **建议加一行 HUD 取证**（不改逻辑）：`zoom / canvas rect / seedMarkerSize / dWorld`，一次截图就能定死所有 px↔wu 换算。
