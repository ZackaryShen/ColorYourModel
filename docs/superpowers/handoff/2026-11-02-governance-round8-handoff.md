# 交接文档：main 治理 + 实验特性 + 多轮验收修复（Round 8）

> 日期：2026-11-02 ｜ 分支：`feature_012_governance` ｜ HEAD：`df094b7`
> 读者：接手继续开发/debug 的新会话。本文档假设你对该会话历史零了解。

---

## 0. 一句话现状

main 治理（剥离 0.2.0）+ 开发者实验特性系统已完成并全部提交；多轮用户验收中，
**种子擦除 bug 经三轮修复失败后已换用"世界距离匹配"方案（df094b7），代码全部测试通过
（vitest 147/147、tsc、esbuild），但尚未经用户实机验收**。此外 main 的 force-push 待用户
最终验收后执行。

---

## 1. 环境与启动（⚠️ 最高优先级的坑）

### 1.1 dev 启动方式 —— 不要用会弹控制台窗口的方式！

**教训（2026-11-01 实锤）**：用 `spawn('cmd.exe', ['/c','npm run tauri dev'], {detached:true})`
且 stdio 重定向/继承时，Windows 会弹出一个控制台窗口。该窗口默认开启 **QuickEdit 模式**：
用户任何一次鼠标点进窗口（哪怕无意单击），控制台就进入文本选择状态，**阻塞所有向它写
stdout 的进程——vite、cargo、连同应用本体一起冻结**（应用日志写它的 stdout）。表现就是
"应用随便操作几下就未响应几分钟，Ctrl+C 后恢复"。之前多轮"未响应"报告全是这个环境问题，
不是应用代码 bug。

**正确启动方式**（当前在用）：

```js
// 用 node 脚本（ctx_execute 或 bash node -e）：
const { spawn } = require('child_process');
const child = spawn('cmd.exe', ['/c', 'npm run tauri dev'], {
  cwd: 'C:/selfDIr/Projects/ColorYourModel-worktrees/feature_012_governance',
  detached: true,
  stdio: 'ignore',     // 丢弃控制台输出——应用内 🐞 日志才是日志面
  windowsHide: true,   // 不弹控制台窗口——无可误点的 QuickEdit 表面
});
child.unref();
```

就绪探针：`curl http://localhost:1420/`（vite 200）+ `tasklist | grep color-your-model` +
PowerShell `(Get-Process color-your-model).Path` 确认是 governance worktree 的构建。

### 1.2 基本路径

| 项 | 路径 |
|---|---|
| 主仓库（020 分支检出） | `C:/selfDIr/Projects/ColorYourModel` |
| **治理 worktree（本会话工作区）** | `C:/selfDIr/Projects/ColorYourModel-worktrees/feature_012_governance` |
| 分支 | `feature_012_governance`（基于 main=d74ce71 之上，领先 ~30 commits） |
| main 的 worktree 占用 | `.../main_roll`（force-push 前别动它） |
| 备份分支 | `main_backup_pre_020_strip` @ 6b2c622 |

### 1.3 验证命令（每次修改后必跑）

```bash
npx tsc --noEmit            # 类型检查
npx vitest run              # 147 个测试（20 文件）
cd src-tauri && cargo test  # 193 个测试（改 Rust 时）
# 快速语法定位可用： npx esbuild src/components/Viewport/Viewport.tsx --loader:.tsx=tsx --log-level=error
```

测试日志里 `[update] progress listen failed` / `resetSegmentation failed backend exploded`
两行是 jsdom 无 Tauri API 的既有噪音，不是失败。

---

## 2. 本会话已完成并验收 ✅

1. **main 治理**：main 严格重置到 `d74ce71`（012 里程碑），23 个误入的 0.2.0 commit 全部
   保留在 `feature_020_convergence`。**force-push origin/main 待用户最终点头**（本地 main
   diverged：ahead 6 / behind 31，用 `git push --force-with-lease origin main`）。
2. **实验特性系统**：`src/experimentalFeatures.ts` 注册表（5 特性，默认全关）+
   `PreferencesDialog`（⚙ 在 StatusBar 与帮助菜单）。store 五件套模式——**注意：自定义
   `mergePrefs` 会静默丢弃未 claim 的持久化字段，新增持久化字段必须去 appStore 底部 claim**。
3. **Issue 移植**：#20/#21/#22/#23/#24/#28/#29 cherry-pick 或手移植到 012 代码基
   （020-only 的 c1ae169/a36841d/6cf5262 等明确跳过，理由见 git log）。
4. **绘制颜色（paint provenance）**：后端 `MeshModel.paint_sources`（face→(source,painted)）；
   brush/spray/smart-brush 记录来源色，填充/橡皮/替换不记录（用户明确要求填充与"泛绘制类"
   分开显示）。命令：`paint_source_groups` / `replace_paint_source_shaped`（保形重涂：最小二乘
   恢复渐变强度再重混）/ `paint_source_faces`（悬停描边）。**用户已验收 ✅**。
5. **已用颜色**：按源色统计活绘制面；默认灰被 `condenseUsedColours` 排除
   （`src/utils/usedColours.ts`）。**用户已验收 ✅**。
6. **播种落点修复（round 5）**：种子改记 face 质心；flat faces 数组索引
   `fi=faceIndex*3` 已验证。**用户已验收 ✅**。
7. 种子图标放大 33%（0.0016×对角线）、擦除/播种 SVG 光标、自动模式禁播提示、
   View 工具 Ctrl 多选描边（SegmentOutline 恢复）、ViewGizmo 纹理自愈、面板间距。

---

## 3. 进行中 ⏳：种子擦除（round 8 世界距离方案，待验收）

### 3.1 现状

**HEAD `df094b7`** 已把擦除匹配改为：**点击命中点（raycast 的世界坐标）直接与各种子存储的
世界坐标比距离**，阈值 = `seedMarkerSize * 2.5`（seedMarkerSize≈0.0016×bbox 对角线，
148k 面模型上约 0.33 世界单位 → 阈值约 0.82wu）。

实现位置：
- `eraseNearWorldPoint` useCallback：`src/components/Viewport/Viewport.tsx` 约 1069-1109 行
- down 分支调用：约 1990 行（`eraseNearWorldPoint(local.point)`）
- move smear：onPointerMove 开头（`raycastFace → eraseNearWorldPoint(hit.point)`）
- 大 effect deps 已含 `eraseNearWorldPoint`

miss 时状态栏显示 `erase miss: nearest seed X.XXwu (threshold 0.82wu)`——X 是命中点到最近
种子的**真实世界距离**，一眼可判。

### 3.2 为什么理论上不可能再偏

红点标记画在种子的世界坐标上；raycast 命中点就是你点击处的模型表面真实坐标。两者都是
模型局部世界的点，**与相机、投影、缩放、平移全部无关**。只要用户点的是红点图形范围内
（±0.33wu），命中点距种子必然 < 阈值。

### 3.3 验收步骤（新会话第一件事）

1. 按 §1.1 启动 dev（或确认已在跑）
2. 导入 STL → 种子工具 → 播 2-3 个种子 → 开"启用种子擦除"
3. 点红点 → 应回退计数并显示"已擦除"；按住划过 → 连续抹除
4. 若仍 miss：贴 `[seed-erase] miss` 日志行 + 状态栏 wu 数值——**wu 数值直接说明命中点
   与种子的真实距离**，据此判断是"命中点根本不在 marker 上"还是别的

### 3.4 三轮失败排查史（⛔ 新会话不要重蹈）

| 轮次 | 方案 | 实测证据 | 结论 |
|---|---|---|---|
| R6 | 屏幕 32px 阈值（`Vector3.project(camera)` 手动投影 + clientX−rect.left） | 完全 miss；HUD 无日志更新 | **project() 投影结果与渲染管线系统性偏差 244px 级**（点击命中 3D 点与种子 3D 点差 <2wu，投影屏幕位差 244px） |
| R7 | 同上 + 坐标修正 + 状态栏诊断 | 全 miss；`nearestSeedScreen` 与 click 差 235-652px 且**不随点击位置线性变化** | 同上，投影不可修 |
| R8-first | 放弃投影，用 picking ray：`raycaster.setFromCamera` 后 `ray.distanceToPoint(seed)/worldPerPx` | 全 miss；distPx=549→13293，随 pan 暴涨——**dWorld 量级 133wu ≈ 整个模型尺寸，射线数据本身就是垃圾**。且 raycastFace 与擦除读的是同一 raycaster 实例（纸面核查过），无法解释 | 相机矩阵参与的一切换算在该场景均不可信（具体根因未查明，别再试屏幕/射线方案） |
| R8-final | **世界距离匹配（当前 df094b7）** | 理论自洽 + 全测试通过 | **待用户验收** |

> 深层教训：这个项目里 three.js 相机相关 API（project/setFromCamera+distanceToPoint）在
> 此正交相机场景给出的结果与渲染管线不一致，具体机制未查明（可能与 R3F orthographic
> camera 的 frustum 管理/drei OrbitControls zoom 有关）。**不要再造第三种屏幕空间方案**。

---

## 4. 已知坑红线（全局适用）

1. **QuickEdit 冻结**（§1.1）——dev 必须隐藏窗口 + stdio ignore。
2. **MeshData 平铺数组**：`faces` 是平铺 `number[]`（3 顶点索引/面），`vertices` 平铺 xyz。
   索引写错 = 播种漂移类 bug（R5 踩过）。
3. **store `mergePrefs` 静默丢弃**未 claim 的持久化字段——新增字段必须 claim，否则每次
   启动重置。
4. **PS5.1/管道传中文乱码**：给 exe 传中文一律走 UTF-8 文件；复杂脚本先 Write 落盘再执行。
5. **python heredoc 里匹配多行代码做替换容易失配**（本会话两次 AssertionError）——优先
   行号 + 前后锚点 assert，改完立即 esbuild/tsc 验证。
6. **大块替换有吃掉闭合括号的风险**（本会话吃掉一个 `}` 造成结构失衡）——替换后必跑
   `npx esbuild ... --log-level=error` 快速抓语法损伤，再 tsc。
7. log.info 签名是 `(scope, message: string, data?)`——**不要用 `as unknown as string` 绕过**
   （R7 踩过）。
8. 编译/打包由用户手动执行；助手只跑 vitest/tsc/cargo/esbuild。
9. i18n key 中英双语进 `src/i18nDict.ts`，store 里存 key 不存字面量。
10. dev 依赖 ：1420 端口；旧 dev 树杀干净再重启（`Get-CimInstance Win32_Process` 过滤
    CommandLine 匹配 tauri/vite，Stop-Process）。

---

## 5. 待办与下一步

1. **用户验收种子擦除**（§3.3）——通过后把 R6/R7/R8 探针代码清一下（可保留 miss 状态栏
   提示，删冗余日志），单独 commit。
2. **全部验收通过后收尾序列**：
   a. 清理临时代码/探针（保留有价值的诊断）
   b. `git push --force-with-lease origin main`（需用户明确点头；本地 main=d74ce71）
   c. 合并 `feature_012_governance` → main（fast-forward，main 是祖先）
   d. 可选：归档/删除 `main_backup_pre_020_strip`
3. 遗留小项：擦除模式按钮文案（激活态文案未变，仅红边框）；suggestedSeeds 在检测器关闭时
   保留（设计决定，非 bug）。
4. 若用户报告新的"未响应"：先按 §1.1 检查是不是又有人用老方式启动了 dev；可用
   `C:/tmp/cpu-sample.ps1`（如还在）采样 rust-backend vs webview 进程 CPU。

---

## 6. 关键文件地图

| 文件 | 内容 |
|---|---|
| `src/components/Viewport/Viewport.tsx`（~3400 行） | onPointerDown 播种/擦除（~1771 起）、onPointerMove smear（~2120）、SeedMarkers（~2680 起，共享几何+单 useFrame）、eraseNearWorldPoint（~1069） |
| `src/store/appStore.ts` | seedPoints 五件套、PersistedPrefs/partialize/mergePrefs（底部） |
| `src/experimentalFeatures.ts` + `PreferencesDialog/` | 实验特性注册表与开关 UI |
| `src/components/ColorPanel/` (+ReplaceColourSection) | 已用颜色/绘制颜色/替换颜色 UI |
| `src/utils/{usedColours,paintQueue,segmentVisibility,resetGate,stlDrop,historyPanel,replaceColour}.ts` | 各子系 |
| `src-tauri/src/commands/segment.rs` | manual_region_add_point（轻量 snap）、finalize_manual_region |
| `src-tauri/src/segment/manual.rs` | snap_point_to_vertex_on_face（k=1.5 吸附半径、60° 同侧） |
| `src-tauri/src/mesh/model.rs` | paint_sources / paint_source_groups / replace_paint_source（~712+） |
| `src/hooks/useTauriCommand.ts` | replacePaintSource→`replace_paint_source_shaped` 等封装 |

## 7. 本会话 commit 链（治理分支，最新在下）

`7b080e9`(View-Ctrl 描边+擦除探针) → `08989bf`(sow 探针) → `5d38d9f`(rAF 心跳) →
`6baa07e`(擦除状态栏诊断) → `b98e869`(射线擦除—已被证伪) → **`df094b7`(世界距离擦除—当前)**

> 诊断探针（sow-diag / freeze-diag / seed-erase 日志）按 §5.1 在验收后清理。
