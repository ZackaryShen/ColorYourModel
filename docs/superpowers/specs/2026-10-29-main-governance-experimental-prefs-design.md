# Main 治理 + 实验性功能偏好设计（2026-10-29）

Status: approved（用户 2026-10-29 确认三个决策：严格版清理 / 检测器+权重滑杆归实验 / StatusBar 齿轮入口）

## 背景

0.2.0 规划（P0 质量地基 → P1 .cym 工程 → P2 渐变/投影/变换/分件）的主线提交在某次合并时被整体带进了
main，而 main 当前应停留在 0.1.2 里程碑（012）。同时，智能分区手动模式暴露了多个实验性控件，
需要「开发者模式 / 实验性功能」偏好入口统一治理。

## A. main 清理（严格版）

- 事实：main 第一父链上 `e679271..c1ae169` 为连续的 0.2.0 工作段（P0：CI 硬门禁、版本号同步工具、
  fuse 耗时埋点、隐藏实验算法 pref、lasso 屏幕空间闭合修复、seed-panel 测试钉住；P1：.cym 工程 +
  工具栏分组；P2：渐变画笔、图片投影、变换 gizmo、分件导出 spike + 文档；外加 stroke_undo 线的
  `c1ae169` viewport 修复）。
- 这段工作已完整存在于 `feature_020_convergence`（020 集成）及其子分支
  （feature_020_p0_quality / p1_project / p2_paint / bugfix_020_stroke_undo），无需迁移。
- 操作：备份分支 `main_backup_pre_020_strip`（= 6b2c622）；main 重置为 `d74ce71`
  （bugfix_012 tip = 0.1.1 + 6 个 012 提交，线性历史，版本号保持 0.1.1）。
- 后果与留路：main 暂时失去 vitest 硬门禁 CI 等工程基建，待 020 合回时一并回归（main ⊂ 020，
  未来合并零冲突）。`c1ae169`（viewport useCallback 修复）如确认是 012 的 bug 可单独 cherry-pick。
- 验证探针：`git diff main feature_012_governance` 为空；`git diff d74ce71 main_backup_pre_020_strip --stat`
  只含 020 代码；被剥提交全部仍被 feature_020_convergence 包含。
- 推送：main 有 origin/main 上游，重置后需 `--force-with-lease` force-push，推送前单独向用户确认。

## B. 实验性功能偏好

### 语义

- 保留项（不受开关影响）：推荐种子 💡（含推荐数量）、手绘点种（点击模型放种子）、手动清除种子
  （擦除模式 / Backspace / Esc / 清空推荐）、自动（融合）模式、眼睛区域保护。
- 实验项（默认隐藏，偏好中勾选启用）：
  1. `seedDetectPlanar` 平面种子检测（检测按钮 + 显隐切换 + 清除 + solo 快捷键）
  2. `seedDetectMultiview` 多视角种子检测（同上）
  3. `seedDetectCrossSection` 截面种子检测（同上）
  4. `seedWeightSliders` 显著性权重滑杆（曲率 / 凹度）

### 隐藏语义（无悬挂、无副作用）

- 检测器关 → SeedPanel 隐藏整行（检测按钮 / 显隐 / 清除 / solo 按钮）；store 中对应 region 池
  立即清空（Viewport BoundaryLines 与提示语随之消失，不留悬空引用）。切换走单一 store action。
  `suggestedSeeds` 不强制清空（用户可用保留的「清空推荐」自行管理，避免勾选框静默删用户数据）。
- 权重滑杆关 → 隐藏两行滑杆；推荐种子按代码默认权重（1.0 / 1.0）调用后端，不泄漏隐藏前
  用户拖过的值（effective-value 降级，同 P0 effectiveResegKind 先例）。

### 结构

- `src/experimentalFeatures.ts`：单一机器可读注册表（id / labelKey / hintKey）、默认值全 false、
  `sanitizeExperimentalFeatures`（未知 id 或非布尔值一律丢弃降级到默认）。
- store 五件套一起动：`AppStore.experimentalFeatures` + `setExperimentalFeature`（检测器关时清池）+
  `PersistedPrefs` + initial + partialize + mergePrefs 显式认领。注意：mergePrefs 会静默丢弃未认领
  字段，五处少一处即等于永远丢档（P0 4e22709 已实证过该坑）。
- UI：新组件 `PreferencesDialog`（ExitConfirmDialog 同款深色模态，Esc / × / 点背景关闭，
  role="dialog"），「偏好设置」标题 +「实验性功能」分组，每项一个勾选框 + 一句说明。
  StatusBar 的 🐞 / 🌙 / EN 按钮排加 ⚙ 打开（App 持有开关状态）。
- i18n：`prefs.*` 与 `exp.*` 键全部 zh / en 双语（M8 约定：store 存 i18n KEY）。

### 测试

- `experimentalFeatures.test.ts`：sanitize 往返（默认 / 未知 id 丢弃 / 非布尔丢弃 / 合法认领）。
- `PreferencesDialog.test.tsx`：默认四项全部未勾选；勾选写入 store；关掉检测器清空对应 region 池。
- `SeedPanel.test.tsx` 追加：默认（实验全关）检测按钮与权重滑杆不渲染、推荐种子仍在；实验全开
  后行恢复；权重滑杆隐藏时 recommendSeeds 收到默认权重（防静默泄漏回归）。
- 验证：`npx vitest run` 全量绿并贴 passed 数；tsc 无错误。Tauri 打包由用户手动执行。
