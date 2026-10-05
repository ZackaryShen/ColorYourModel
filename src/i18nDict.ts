/**
 * Translation dictionary + pure translate() helpers.
 *
 * This module must stay dependency-free (no store imports) so the Zustand
 * store can call translate() without an import cycle: i18n.ts (useT) →
 * appStore → i18nDict. The React hook wrapper lives in i18n.ts.
 */
export type Lang = "zh" | "en";

// ─── Translation dictionaries ────────────────────────────────────
const dict: Record<string, Record<Lang, string>> = {
  // Toolbar — buttons
  "toolbar.import": { zh: "导入 STL 文件", en: "Import STL file" },
  "toolbar.export": { zh: "导出涂装模型为 3MF", en: "Export painted model as 3MF" },
  "toolbar.reSegment": { zh: "重新自动分区（阈值: 30°）", en: "Re-run auto segmentation (threshold: 30°)" },
  "toolbar.smartSegment": { zh: "智能分区（SDF 形态直径）", en: "Smart segment (SDF shape diameter)" },
  "toolbar.undo": { zh: "撤销（Ctrl+Z）", en: "Undo (Ctrl+Z)" },
  "toolbar.redo": { zh: "重做（Ctrl+Y）", en: "Redo (Ctrl+Y)" },
  "lasso.hint": { zh: "套索：点击顶点选点，回到起点闭合（青色=吸附落点）。Backspace 删点，Ctrl+Z 撤销，Esc 取消", en: "Lasso: click vertices; click start to close (cyan = snap target). Backspace removes a point, Ctrl+Z undoes, Esc cancels" },
  "lasso.need3": { zh: "至少需要 3 个点才能闭合区域", en: "Need at least 3 points to close a region" },
  "lasso.cancelled": { zh: "已取消当前套索", en: "Lasso cancelled" },
  "lasso.undoPoint": { zh: "已撤销一个选点", en: "Removed last point" },
  "toolbar.importComplete": { zh: "导入完成", en: "Import complete" },
  "status.projectSaved": { zh: "工程已保存", en: "Project saved" },
  "toolbar.importFailed": { zh: "导入失败", en: "Import failed" },
  "toolbar.openProjectFailed": { zh: "打开工程失败", en: "Open project failed" },
  "toolbar.saveProjectFailed": { zh: "保存工程失败", en: "Save project failed" },
  "toolbar.saveProject": { zh: "保存工程（.cym）", en: "Save project (.cym)" },
  "toolbar.groupPaint": { zh: "涂色工具", en: "Paint tools" },
  "toolbar.groupSegment": { zh: "分区工具", en: "Segment tools" },
  "toolbar.groupCollapse": { zh: "折叠/展开分组", en: "Collapse/expand group" },
  "toolbar.startImport": { zh: "开始导入…", en: "Starting import…" },

  // Toolbar — tool tooltips (differentiated descriptions)
  "tool.view": { zh: "🖐️ 查看/导航 — 左键旋转、右键平移、中键缩放；选工具后按住 Alt 也可旋转", en: "🖐️ View / Navigate — left rotate, right pan, middle zoom; hold Alt to rotate while a tool is active" },
  "tool.fill": { zh: "🪣 填充 — 点击任意面，一键填满整个分区", en: "🪣 Fill — click any face to fill entire connected region" },
  "tool.brush": { zh: "🖌️ 画笔 — 平滑连续涂色，边缘渐变过渡", en: "🖌️ Brush — smooth continuous paint with edge falloff" },
  "tool.gradient": { zh: "🌈 渐变画笔 — 沿笔划从颜色 A 渐变到颜色 B", en: "🌈 Gradient brush — fades from color A to color B along the stroke" },
  "tool.spray": { zh: "💨 喷罐 — 随机散点喷涂，模拟真实喷漆颗粒感", en: "💨 Spray — random scatter dots, simulates real spray-can texture" },
  "tool.smart": { zh: "🎯 智能笔 — 自动识别分区边界，不会涂出区域", en: "🎯 Smart — segment-aware, stays within region boundary" },
  "tool.eyedropper": { zh: "💧 吸管 — 从模型表面拾取颜色", en: "💧 Eyedropper — pick color from model surface" },
  "tool.eraser": { zh: "🧹 橡皮 — 擦除颜色，恢复为默认底色", en: "🧹 Eraser — restore the default base color" },
  "tool.segment": { zh: "✂️ 分区画笔 — 拖拽涂选面，松开鼠标创建分区", en: "✂️ Segment Brush — drag to paint faces, release to create region" },
  "tool.lasso": { zh: "📍 选点套索 — 依次点选顶点围合区域，点击起点闭合", en: "📍 Lasso — click vertices to outline a region; click the start point to close" },
  "tool.seed": { zh: "🌱 种子分区 — 在模型上点选若干种子，算法按几何智能长成区域", en: "🌱 Seed — drop seed points; the algorithm grows each into a region by geometry" },
  // Short tool names for the status bar (the tool.* keys above are long
  // tooltips with emoji — wrong register for `工具: <name>`). Keys follow the
  // PaintTool enum values verbatim, including `picker` for the Eyedropper.
  "toolName.view": { zh: "视图", en: "View" },
  "toolName.fill": { zh: "填充", en: "Fill" },
  "toolName.brush": { zh: "画笔", en: "Brush" },
  "toolName.gradient": { zh: "渐变画笔", en: "Gradient brush" },
  "toolName.spray": { zh: "喷罐", en: "Spray" },
  "toolName.smart": { zh: "智能笔", en: "Smart" },
  "toolName.picker": { zh: "吸管", en: "Eyedropper" },
  "toolName.eraser": { zh: "橡皮", en: "Eraser" },
  "toolName.segment": { zh: "分区画笔", en: "Segment Brush" },
  "toolName.lasso": { zh: "套索", en: "Lasso" },
  "toolName.seed": { zh: "种子", en: "Seed" },
  "seed.hint": { zh: "种子分区：在模型上点击放置种子（每点一个区域），调节屏障角度后点「生长」。未点的区域由最近种子兜底。Backspace/Esc 清空。", en: "Seed: click to place seeds (one region each), tune the barrier angle, then Grow. Un-seeded patches fall back to the nearest seed. Backspace/Esc clears." },
  "seed.barrier": { zh: "屏障角度 (°)", en: "Barrier angle (°)" },
  "seed.barrierHint": { zh: "大于此二面角的棱成为硬边界，区域不会越过折缝", en: "Edges above this dihedral angle are hard boundaries the regions won't cross" },
  "seed.optimizer": { zh: "智能优化（合并微小区域）", en: "Optimize (merge tiny regions)" },
  "seed.grow": { zh: "🌱 生长", en: "🌱 Grow" },
  "seed.modeAuto": { zh: "自动（融合）", en: "Auto (fuse)" },
  "seed.modeManual": { zh: "手动（生长）", en: "Manual (grow)" },
  "seed.modeAutoTitle": { zh: "自动生成：全局边级投票，一键分区，无需放置种子", en: "Auto: global edge-vote, one-click partition, no seeds needed" },
  "seed.modeManualTitle": { zh: "手动生长：从你放置/推荐/检测出的种子做测地生长", en: "Manual: geodesic grow from your placed / suggested / detected seeds" },
  "seed.modeAutoHint": { zh: "当前：自动融合——点击「🧩 融合生成」按折角+平面+多视角全局投票切出分区，眼睛区域会被自动保护。", en: "Now: auto-fuse — click 🧩 Fuse & generate to cut the partition by dihedral+planar+MultiView global vote; eye regions are auto-protected." },
  "seed.modeManualHint": { zh: "当前：手动生长——先放置/推荐/检测种子，再点「🌱 生长」做测地分水岭；平面/多视角/截面的建议种子也会并入。", en: "Now: manual grow — place / suggest / detect seeds, then click 🌱 Grow for geodesic watershed from the nearest seed; planar/MultiView/cross-section suggestions also feed in." },
  "seed.clear": { zh: "清空种子", en: "Clear seeds" },
  "seed.count": { zh: "已放置 {0} 个种子", en: "{0} seeds placed" },
  "seed.needOne": { zh: "请至少放置一个种子", en: "Place at least one seed first" },
  "seed.done": { zh: "种子分区完成：{0} 个区域", en: "Seed partition done: {0} regions" },
  "seed.eraseMode": { zh: "擦除模式", en: "Eraser" },
  "seed.closeTitle": {
    zh: "关闭 Seed 面板（Esc / 切回 View 工具；已放置的种子会被保留）",
    en: "Close Seed panel (Esc / switch back to View tool; placed seeds are kept)",
  },
  "seed.eraseHint": { zh: "擦除模式：点击模型上任一红色种子即可删除它（只删那一个，不清空全部）。再点「擦除模式」退出。", en: "Eraser: click any red seed on the model to delete just that one (not all). Click Eraser again to exit." },
  "seed.erased": { zh: "已擦除 1 个种子（剩余 {0} 个）", en: "Erased 1 seed ({0} left)" },
  "seed.eraseMiss": { zh: "附近没有种子可擦除（请点在种子圆点上）", en: "No seed nearby to erase (click on a seed dot)" },
  "seed.suggestCount": { zh: "推荐数量", en: "Suggest count" },
  "seed.weightCurv": { zh: "显著性·曲率", en: "Significance · curvature" },
  "seed.weightCurvHint": { zh: "权重越高，推荐点越偏向锐利折缝（硬边界）附近的内部", en: "Higher weight pushes suggestions toward interiors near sharp creases" },
  "seed.weightConc": { zh: "显著性·凹度", en: "Significance · concavity" },
  "seed.weightConcHint": { zh: "权重越高，推荐点越偏向凹谷（真实零件分界）附近的内部", en: "Higher weight pushes suggestions toward interiors near concave valleys" },
  "seed.suggest": { zh: "💡 推荐种子", en: "💡 Suggest seeds" },
  "seed.clearSuggest": { zh: "清空推荐 ({0})", en: "Clear suggestions ({0})" },
  "seed.planar": { zh: "🟦 平面种子", en: "🟦 Planar seeds" },
  "seed.clearPlanar": { zh: "清空平面 ({0})", en: "Clear planes ({0})" },
  "seed.multiview": { zh: "👁 多视角", en: "👁 MultiView" },
  "seed.clearMultiview": { zh: "清空多视角 ({0})", en: "Clear MultiView ({0})" },
  "seed.crossSection": { zh: "✂️ 截面", en: "✂️ Cross-section" },
  "seed.clearCrossSection": { zh: "清空截面 ({0})", en: "Clear cross-sections ({0})" },
  "seed.clearPlanarTitle": { zh: "清空 Layer 1 平面数据", en: "Clear Layer 1 planar data" },
  "seed.clearMultiviewTitle": { zh: "清空 Layer 3 多视角数据", en: "Clear Layer 3 MultiView data" },
  "seed.clearCrossSectionTitle": { zh: "清空 Layer 2 截面数据", en: "Clear Layer 2 cross-section data" },
  "seed.hide": { zh: "隐藏此层", en: "Hide this layer" },
  "seed.show": { zh: "显示此层", en: "Show this layer" },
  "seed.showAll": { zh: "全部显示", en: "Show all" },
  "seed.soloPlanar": { zh: "只显 Layer 1 平面", en: "Solo: Layer 1 planar only" },
  "seed.soloMultiview": { zh: "只显 Layer 3 多视角", en: "Solo: Layer 3 MultiView only" },
  "seed.soloCrossSection": { zh: "只显 Layer 2 截面", en: "Solo: Layer 2 cross-section only" },
  "seed.fuse": { zh: "🧩 融合生成", en: "🧩 Fuse & generate" },
  "seed.fuseTitle": { zh: "把 Layer 0 折角 + Layer 1 平面 + Layer 3 多视角的区域成员关系按边级多数投票融合成最终分区并落盘（不再依赖种子点）", en: "Fuse Layer 0 dihedral + Layer 1 planar + Layer 3 MultiView region membership by edge-level majority vote into the final partition (no seed points needed)" },
  "seed.fuseDihedral": { zh: "折角阈值", en: "Dihedral °" },
  "seed.fuseDihedralHint": { zh: "融合的几何主干：按二面角在折痕处切割。光滑/单色模型上平面与多视角都没信号，靠它才能切出部件。雕刻类手办（哥斯拉/宠物等）折角被摊平，建议 2-3° 才能分出头/四肢/尾巴；硬表面模型 5-15° 即可。越低切得越细，越高越粗", en: "Geometry backbone for the fuse: cut at dihedral creases. On smooth/single-colour meshes planar + MultiView have no signal, so this is what splits the model. Sculpted figures (godzilla/creatures) spread their folds below 5° — use 2-3° to separate head/limbs/tail; hard-surface models are fine at 5-15°. Lower = finer, higher = coarser" },
  "seed.clearAll": { zh: "🗑 清空分区", en: "🗑 Reset partition" },
  "seed.clearAllTitle": { zh: "清空分区与上色（回到刚导入的干净状态，不可撤销）", en: "Clear partition + paint (back to the freshly-loaded clean state; cannot be undone)" },
  "seed.fuseHint": {
    zh: "「融合生成」把 Layer 1 平面 + Layer 3 多视角的区域成员关系按边级多数投票融合成最终分区并直接落盘（可撤销）。每条边由两算法投票「该不该切开」，切票多于留票才切开，平票合并——抑制过切、让分区更成块。无需手动种种子。",
    en: "Fuse & generate merges Layer 1 planar + Layer 3 MultiView region membership by edge-level majority vote into the final partition and commits it (undoable). Each edge is voted cut/keep by the two algorithms; cut wins only when it outvotes keep, ties merge — suppressing over-splitting into more meaningful blocks. No manual seeds needed.",
  },
  "seed.crossSectionHint": {
    zh: "「截面」用射线/切片（Layer 2，docs/09）沿主轴逐层切模型，找出剖面变化剧烈处（特征截面），用绿色线条标出真实切面轮廓——纯视觉证据，不生成分区种子（截面是平面而非面，Layer 2 不参与裁决）。含广义绕数内壁置信度。阈值：每轴 24 切片、变化≥最剧烈处的一半。",
    en: "Cross-section uses ray/marching-plane (Layer 2, docs/09): it slices the model along each principal axis and marks where the cross-sectional profile changes sharply (feature cross-sections), drawing the real slice contours in green — purely visual evidence, it does NOT create partition seeds (a slice is a plane, not a face; Layer 2 is not a verdict). Includes a generalized-winding-number inside/outside confidence. Thresholds: 24 slices/axis, change ≥ half the sharpest.",
  },
  "seed.multiviewHint": {
    zh: "「多视角」用 MultiView 3→2→3（Layer 3，docs/09）提供第二种意见：从多个视角投影→2D连通区域生长→回投→带权 match graph 切割。每个共识簇生成一枚建议种子（粉色幽灵，可点击采纳），边界用橙色线条标出。阈值：12 视角、法向差 20°、≥1 视角一致才合并。",
    en: "MultiView uses 3→2→3 (Layer 3, docs/09) as a second opinion: project from many views → grow 2D-connected regions → back-project → cut a weighted match graph. Each consensus cluster yields a suggested seed (magenta ghost, click to accept), outlined in orange. Thresholds: 12 views, 20° normal diff, ≥1 view agreement to merge.",
  },
  "seed.planarHint": {
    zh: "「平面种子」自动检测模型上的连续平面区域（Layer 1，docs/09）：每个平面生成一枚建议种子（粉色幽灵，可点击采纳）并用青色线条标出其边界。阈值：法向差 15°、平面距离 M/30。",
    en: "Planar seeds auto-detect the mesh's continuous flat patches (Layer 1, docs/09): each plane yields one suggested seed (magenta ghost, click to accept) and its boundary is outlined in cyan. Thresholds: 15° normal diff, M/30 plane distance.",
  },
  "seed.suggestHint": {
    zh: "亮粉色十字标记为推荐种子（A2 修复后已可见）：点「幽灵附近」采纳该条；点「远离所有幽灵」则作为手动种子继续添加（两条路径并存）。正式种子仍为黄色/青色。",
    en: "Magenta cross = suggested seed (visible since the A2 fix). Click CLOSE to one cross to accept it; click FAR from all crosses to add a new manual seed (both paths work side-by-side). Real seeds stay yellow/cyan.",
  },
  "seed.accepted": { zh: "已接受推荐种子（共 {0} 个）", en: "Accepted suggestion ({0} seeds total)" },
  "seed.acceptDup": { zh: "该推荐点附近已有种子，已忽略", en: "A seed already exists near this suggestion; ignored" },
  "seed.eye": { zh: "👁 眼睛识别", en: "👁 Eye detect" },
  "seed.eyeAuto": { zh: "👁 自动识别眼睛", en: "👁 Auto eye detect" },
  "seed.eyeAutoTitle": { zh: "Stage 1: 全网格扫描对称的眼睛凸起并自动分区", en: "Stage 1: scan the whole mesh for symmetric eye bumps" },
  "seed.clearEye": { zh: "清空眼睛 ({0})", en: "Clear eye ({0})" },
  "seed.eyeTitle": { zh: "清空 Layer 5 眼睛语义区域", en: "Clear Layer 5 eye regions" },
  "seed.eyeHint": {
    zh: "「眼睛识别」(Layer 5, docs/10) 对**选中的分区**做 ROI 语义分解：把包围眼睛的那块面片细分成 眼球(Globe)/眼白(Sclera,启发式低置信)/眼睑(Eyelid)/眼眶(Socket) 四个子区域，并用分色边界线标出。先用任意算法或套索圈出一个包围眼睛的分区，选中它，再点此按钮。阈值与判定见 docs/10。",
    en: "Eye detect (Layer 5, docs/10) does a semantic ROI decomposition of the CURRENTLY SELECTED partition: it splits the eye-bounding faces into Globe / Sclera (heuristic, low confidence) / Eyelid / Socket sub-regions and outlines each in its own colour. First box an eye area with any algorithm or the lasso, select that partition, then click here. Thresholds per docs/10.",
  },
  "seed.eyeNeedSegment": { zh: "请先在分区面板选中一个包围眼睛的分区", en: "Select a partition that bounds an eye first" },
  "seed.eyeNeedFaces": { zh: "该分区面数过少，无法识别眼睛", en: "This partition has too few faces to detect an eye" },
  "seed.eyeNeedSegmentShort": { zh: "请先选中一个包围眼睛的分区（先 Fuse 一次再点其中一块高亮）", en: "Select an eye-adjacent partition first (Fuse then click one region to highlight it)" },
  "seed.eyeNeedSegmentHint": { zh: "👁 眼睛识别需要先有一个「选中分区」：先点「🧩 融合生成」得到几个区域，再用鼠标点其中接近眼睛的那一块让它高亮，最后回来点「眼睛识别」就能跑。", en: "👁 Eye detect needs a selected partition: click \"Fuse & generate\" first, then click any one region in the viewport to highlight it as selected, then click \"Eye detect\" here." },
  "seed.eyeNeedBodySeeds": { zh: "👁 眼睛区域只能作为「保护区」参与生长。请先放置几个身体种子，或使用「融合生成」。", en: "👁 Eye regions can only act as a reservation. Place a few body seeds first, or use Fuse & generate." },
  "seed.eyePickHint": { zh: "👁 想识别眼睛？点 [🖱 点选分区] 进入点选模式，鼠标在模型上点击任一区域即可把它设为选中分区（一次命中即自动退出）；空处点击取消选中。", en: "👁 For eye detect: click [🖱 Pick a partition] to enter pick mode, then click any region on the model to set it as the selected partition (auto-exits on first hit); click empty space to clear." },
  "seed.pickMode": { zh: "🖱 点选分区", en: "🖱 Pick a partition" },
  "seed.pickModeActive": { zh: "🖱 点选模式（点选下一面）", en: "🖱 Pick mode (click next face)" },
  "seed.picked": { zh: "已选中分区 #{0} — 现在可点「眼睛识别」", en: "Selected partition #{0} — you can now click \"Eye detect\"" },
  "seed.pickMiss": { zh: "该位置没有分区——需要先 Fuse（或加载分区视图）", en: "No partition under the cursor — run Fuse or switch to segment view first" },
  "seed.pickCleared": { zh: "已清空选中分区", en: "Selection cleared" },
  "seed.pickPanelTitle": { zh: "标题栏拖动 / 双击重置位置", en: "Drag title bar to move / double-click to reset position" },
  "seed.panelResetTitle": { zh: "把面板复位到屏幕中下方", en: "Reset panel to bottom-center" },
  "seed.panelReset": { zh: "📍 复位", en: "📍 Reset" },
  "seed.soloEye": { zh: "只显 Layer 5 眼睛", en: "Solo: Layer 5 eye only" },

  // BrushSettings
  "brush.title": { zh: "画笔设置", en: "Brush Settings" },
  "gradient.mode": { zh: "渐变模式", en: "Gradient mode" },
  "gradient.path": { zh: "路径", en: "Path" },
  "gradient.radial": { zh: "径向", en: "Radial" },
  "gradient.colorA": { zh: "起点色", en: "Start color" },
  "gradient.colorB": { zh: "终点色", en: "End color" },
  "gradient.length": { zh: "渐变长度（视口对角 %）", en: "Fade length (% of viewport diagonal)" },
  "brush.radius": { zh: "半径", en: "Radius" },
  "brush.strength": { zh: "强度", en: "Strength" },
  "brush.falloff": { zh: "衰减", en: "Falloff" },
  "brush.smooth": { zh: "平滑", en: "Smooth" },
  "brush.linear": { zh: "线性", en: "Linear" },
  "brush.step": { zh: "阶梯", en: "Step" },
  "shading.mode": { zh: "着色模式", en: "Shading Mode" },
  "shading.flat": { zh: "🎯 平整色（精确）", en: "🎯 Flat (exact color)" },
  "shading.shaded": { zh: "💡 有光影（有形体）", en: "💡 Shaded (3D shape)" },

  // ColorPanel
  "color.title": { zh: "颜色", en: "Color" },
  "color.amsPalette": { zh: "AMS 调色板", en: "AMS Palette" },

  // SegmentsPanel
  "segments.title": { zh: "分区", en: "Regions" },
  "segments.reset": { zh: "重置", en: "Reset" },
  "segments.resetHint": { zh: "清除全部分区与上色，回到刚导入的未上色状态", en: "Clear all regions and paint, back to the freshly-loaded uncoloured state" },
  "segments.resetConfirm": { zh: "确定重置分区并清除所有上色吗？此操作不可撤销。", en: "Reset all regions and clear all paint? This cannot be undone." },
  "segments.empty": { zh: "暂无分区，请先导入模型", en: "No segments yet. Import a model first." },
  "segments.renameHint": { zh: "双击重命名，Ctrl+点击多选", en: "Double-click to rename, Ctrl-click to multi-select" },
  "segments.mergeInto": { zh: "合并 {0} 个分区到「{1}」", en: "Merge {0} regions into \"{1}\"" },
  "segments.merged": { zh: "已合并 {0} 个分区（{1} 个面）", en: "Merged {0} regions ({1} faces)" },
  "segments.split": { zh: "拆分", en: "Split" },
  "segments.splitHint": { zh: "沿内部棱角把一个分区拆成多个", en: "Split a region into pieces along its internal creases" },
  "segments.splitThreshold": { zh: "棱角阈值 (°)", en: "Crease threshold (°)" },
  "segments.splitAlongCreases": { zh: "按棱角拆分", en: "Split along creases" },
  "segments.splitDone": { zh: "已拆分：{0} 个面移到新分区", en: "Split: {0} faces moved to a new region" },
  "segments.resegment": { zh: "再切", en: "Re-cut" },
  "segments.resegmentHint": { zh: "只用某算法对这个分区内部重新切分（适合凸分解切不开的管状肢体）", en: "Re-run an algorithm inside this region only (good for tubular limbs the convex decomposition left whole)" },
  "segments.resegmentRun": { zh: "重新切分", en: "Re-segment" },
  "segments.resegmentDone": { zh: "再分区完成：{0} 个新区域", en: "Re-segmented into {0} new regions" },
  "segments.resegmentAlgo": { zh: "算法", en: "Algorithm" },

  // Viewport — ControlsHelp
  "controls.leftPaint": { zh: "🖱️ 左键: 旋转(查看) / Alt+左键: 旋转", en: "🖱️ Left: Rotate (View) / Alt+Left: Rotate" },
  "controls.rightRotate": { zh: "🔄 右键+拖: 平移", en: "🔄 Right+Drag: Pan" },
  "controls.middlePan": { zh: "✋ 中键+拖: 缩放", en: "✋ Middle+Drag: Zoom" },
  "controls.scrollZoom": { zh: "🔍 滚轮: 缩放", en: "🔍 Scroll: Zoom" },
  "controls.viewHint": { zh: "左键 旋转 · 右键 平移 · 中键 缩放 · 滚轮 缩放", en: "Left Rotate · Right Pan · Middle Zoom · Scroll Zoom" },
  "controls.brushHint": { zh: "模型上 左键绘制 · 空白处左键旋转 · 右键 平移 · Ctrl+滚轮 调笔刷", en: "On model: Left paints · On empty: Left rotates · Right Pan · Ctrl+Wheel brush size" },
  "controls.editHint": { zh: "左键 绘制/选取 · 右键 平移 · 中键 缩放 · 滚轮 缩放", en: "Left Paint/Select · Right Pan · Middle Zoom · Scroll Zoom" },

  // Viewport — SegmentToggle
  "view.paintView": { zh: "🎨 涂色视图", en: "🎨 Paint View" },
  "view.segmentView": { zh: "🗺️ 分区视图", en: "🗺️ Segment View" },
  "view.switchToPaint": { zh: "切换到涂色视图", en: "Switch to paint view" },
  "view.showSegments": { zh: "显示分区着色", en: "Show segment regions" },
  // Post-fuse segment-view suggestion card (Viewport top-centre)
  "view.segmentHint": {
    zh: "分区完成：切换到分区视图查看效果更直观",
    en: "Partition ready — the segment view shows it best",
  },
  "view.segmentHintSwitch": { zh: "切换到分区视图", en: "Switch to segment view" },
  "view.segmentHintStay": { zh: "留在当前视图", en: "Stay here" },

  // Viewport — misc
  "view.loading": { zh: "加载中…", en: "Loading…" },
  "view.emptyState": { zh: "导入 STL 文件开始涂装", en: "Import an STL file to get started" },

  // Viewport — lasso / segment-brush status details
  "lasso.selected": { zh: "套索：已选 {0} 个点", en: "Lasso: {0} points selected" },
  "lasso.closeOrEnter": { zh: "（点击起点附近闭合，或按 Enter）", en: " (click near the start point to close, or press Enter)" },
  "lasso.pointsLeft": { zh: "（剩 {0} 个点）", en: " ({0} points left)" },
  "segBrush.commit": { zh: "✏️ 分区笔 face={0} → label={1}", en: "✏️ Segment brush face={0} → label={1}" },
  "brush.sizeStatus": { zh: "笔刷半径 {0}mm（Ctrl+滚轮）", en: "Brush radius {0}mm (Ctrl+Wheel)" },

  // StatusBar
  "status.tool": { zh: "工具", en: "Tool" },
  "status.faces": { zh: "面数", en: "Faces" },
  // `status.tip` was removed (GUI audit 2026-10-02, B4): a static
  // "Left: Rotate (View) | …" string contradicted every non-view tool.
  // StatusBar now resolves its hint through utils/controlsHint.ts, the same
  // source as the viewport help bar (controls.viewHint / brushHint / editHint).
  "status.ready": { zh: "就绪", en: "Ready" },
  "status.loaded": { zh: "已加载 {0} 个面", en: "Loaded {0} faces" },

  // Debug log viewer (iteration 59) + its status-bar toggle
  "debugLog.title": { zh: "🐞 调试日志（{0} 条）", en: "🐞 Debug log ({0} entries)" },
  "debugLog.clear": { zh: "清空", en: "Clear" },
  "debugLog.empty": { zh: "暂无日志", en: "No logs yet" },
  "debugLog.openTitle": { zh: "调试日志 (Ctrl+Shift+L)", en: "Debug log (Ctrl+Shift+L)" },

  // Debug HUD (bottom-left paint-diagnostics overlay)
  "debugHud.collapse": { zh: "收起诊断信息", en: "Collapse diagnostics" },
  "debugHud.expand": { zh: "展开诊断信息", en: "Expand diagnostics" },

  // Language
  "lang.switch": { zh: "EN", en: "中" },

  // Theme (iteration 20)
  "theme.toggle": { zh: "切换深浅色", en: "Toggle theme" },
  "theme.dark": { zh: "🌙 深色", en: "🌙 Dark" },
  "theme.light": { zh: "☀️ 浅色", en: "☀️ Light" },

  // Export dialog (preset library)
  "export.title": { zh: "导出模型", en: "Export Model" },
  "export.format": { zh: "格式", en: "Format" },
  "export.format3mf": { zh: "3MF（切片软件）", en: "3MF (slicer)" },
  "export.formatObj": { zh: "OBJ（带颜色）", en: "OBJ (with colour)" },
  "export.objNote": { zh: "OBJ 直接写入每面 RGB 颜色（.obj + .mtl），无需选择机型/工艺。", en: "OBJ writes per-face RGB directly (.obj + .mtl), no machine/process needed." },
  "export.machine": { zh: "厂商 / 机型", en: "Vendor / Machine" },
  "export.nozzle": { zh: "喷嘴直径", en: "Nozzle diameter" },
  "export.process": { zh: "工艺（打印参数）", en: "Process (print profile)" },
  "export.filament": { zh: "耗材（每槽位）", en: "Filament (per slot)" },
  "export.slots": { zh: " 个槽位", en: " slots" },
  "export.selectAll": { zh: "全选槽位", en: "Select all" },
  "export.deselectAll": { zh: "清除选择", en: "Clear selection" },
  "export.applyToSelected": { zh: "套用到所选 ({0})", en: "Apply to selected ({0})" },
  "export.applyToAll": { zh: "一键套用全部", en: "Apply to all" },
  "export.slotSelect": { zh: "选择槽位 {0}", en: "Select slot {0}" },
  "export.target": { zh: "目标切片器", en: "Target slicer" },
  "export.targetSnapmaker": { zh: "Snapmaker Orca", en: "Snapmaker Orca" },
  "export.targetOrca": { zh: "OrcaSlicer（通用）", en: "OrcaSlicer (generic)" },
  "export.cancel": { zh: "取消", en: "Cancel" },
  "export.confirm": { zh: "选择保存位置并导出", en: "Choose location & export" },
  "export.exporting": { zh: "导出中…", en: "Exporting…" },
  "export.complete": { zh: "导出完成", en: "Export complete" },
  "export.success": { zh: "导出成功：{0}", en: "Exported: {0}" },
  "export.successObj": { zh: "导出成功（含 .mtl 材质）：{0}", en: "Exported (with .mtl): {0}" },
  "export.failure": { zh: "导出失败：{0}", en: "Export failed: {0}" },
  "export.presetLoadFailed": { zh: "导出配置加载失败：{0}", en: "Failed to load export presets: {0}" },
  "export.needModel": { zh: "请先导入模型", en: "Import a model first" },
  "export.noModel": { zh: "尚未导入模型", en: "No model loaded" },
  "export.noModelHint": { zh: "导出需要先导入 STL 模型并完成涂装，请先从「文件 → 导入 STL…」或左侧工具栏导入。", en: "Exporting needs an imported (and painted) STL model first — use File → Import STL… or the left toolbar." },

  // Exit confirmation (issue #7)
  "exit.confirmTitle": { zh: "未保存的涂装工作", en: "Unsaved painting work" },
  "exit.stay": { zh: "留在应用", en: "Stay" },
  "exit.quit": { zh: "直接退出", en: "Quit anyway" },
  "exit.confirmBody": {
    zh: "模型上有尚未导出的涂装/分区修改，退出将会丢失。确定要退出吗？",
    en: "This model has painting or segmentation changes that have not been exported yet. Quit anyway?",
  },

  // View gizmo (Orca-style view cube, bottom-right of the viewport)
  "gizmo.top": { zh: "顶部视图（XY 平面）", en: "Top view (XY plane)" },
  "gizmo.bottom": { zh: "底部视图（XY 平面）", en: "Bottom view (XY plane)" },
  "gizmo.front": { zh: "前视图（XZ 平面）", en: "Front view (XZ plane)" },
  "gizmo.back": { zh: "后视图（XZ 平面）", en: "Back view (XZ plane)" },
  "gizmo.right": { zh: "右视图（YZ 平面）", en: "Right view (YZ plane)" },
  "gizmo.left": { zh: "左视图（YZ 平面）", en: "Left view (YZ plane)" },
  // Short direction words baked into the cube faces (Orca-style)
  "gizmo.face.top": { zh: "顶部", en: "TOP" },
  "gizmo.face.bottom": { zh: "底部", en: "BOTTOM" },
  "gizmo.face.front": { zh: "正面", en: "FRONT" },
  "gizmo.face.back": { zh: "背面", en: "BACK" },
  "gizmo.face.right": { zh: "右侧", en: "RIGHT" },
  "gizmo.face.left": { zh: "左侧", en: "LEFT" },

  // Top menu bar (File / View / Help)
  "menu.file": { zh: "文件", en: "File" },
  "menu.view": { zh: "视图", en: "View" },
  "menu.help": { zh: "帮助", en: "Help" },
  "menu.import": { zh: "导入 STL…", en: "Import STL…" },
  "menu.openProject": { zh: "打开工程…", en: "Open project…" },
  "menu.saveProject": { zh: "保存工程", en: "Save project" },
  "menu.export": { zh: "导出涂装模型…", en: "Export painted model…" },
  "menu.quit": { zh: "退出", en: "Quit" },
  "menu.resetView": { zh: "重置视角", en: "Reset view" },
  "menu.quitFailed": { zh: "退出失败：窗口关闭被拒绝，详情见日志", en: "Quit failed: window close was denied — see the log" },
  // Describes the action in the CURRENT language (the menu must read fully
  // localized in both languages); the StatusBar tooltip keeps the
  // target-language convention, which fits a one-word toggle better.
  "menu.language": { zh: "切换到 English", en: "Switch to Chinese" },
  "menu.themeToLight": { zh: "切换到浅色主题", en: "Switch to light theme" },
  "menu.themeToDark": { zh: "切换到深色主题", en: "Switch to dark theme" },
  "menu.debugLog": { zh: "调试日志 (Ctrl+Shift+L)", en: "Debug log (Ctrl+Shift+L)" },
  "menu.wiki": { zh: "文档总览（Wiki）", en: "Documentation (Wiki)" },
  "menu.userGuide": { zh: "用户手册", en: "User Guide" },
  "menu.examples": { zh: "示例画廊", en: "Example Gallery" },
  "menu.discussions": { zh: "GitHub 讨论区", en: "GitHub Discussions" },
  "menu.about": { zh: "关于 ColorYourModel", en: "About ColorYourModel" },
  "menu.checkUpdates": { zh: "检查更新…", en: "Check for updates…" },

  // About dialog
  "about.desc": {
    zh: "给 3D 打印手办上色的桌面工具：导入 STL，智能分区，涂色后导出带颜色的 3MF/OBJ。",
    en: "A desktop tool for painting 3D-print figures: import an STL, partition it smartly, paint, and export coloured 3MF/OBJ.",
  },
  "about.version": { zh: "版本", en: "Version" },
  "about.tech": { zh: "技术栈", en: "Built with" },
  "about.license": { zh: "开源协议", en: "License" },
  "about.repo": { zh: "源码仓库", en: "Repository" },
  "about.showExperimental": {
    zh: "显示实验性分区算法（sdfGraphCut / 凹度）",
    en: "Show experimental segmentation algorithms (sdfGraphCut / concavity)",
  },
  "about.close": { zh: "关闭", en: "Close" },
  "update.autoCheckToggle": { zh: "启动时检查更新", en: "Check for updates on startup" },

  // Update module (GitHub Releases check / download / passive install).
  // Versions arrive pre-formatted as "vX.Y.Z"; sizes arrive as "N.N" (MB).
  "update.availableTitle": { zh: "发现新版本 {0}", en: "Update available: {0}" },
  "update.currentVersion": { zh: "当前版本 {0}", en: "Current version {0}" },
  "update.installerSize": { zh: "安装包 {0} MB", en: "{0} MB installer" },
  "update.unsavedWorkHint": {
    zh: "更新会关闭应用，未导出的涂装将不会保留。",
    en: "Updating closes the app — unexported paint will be lost.",
  },
  "update.updateNow": { zh: "立即更新", en: "Update now" },
  "update.goToRelease": { zh: "前往发布页", en: "Open releases page" },
  "update.later": { zh: "稍后", en: "Later" },
  "update.skipVersion": { zh: "跳过此版本", en: "Skip this version" },
  "update.neverRemind": { zh: "不再提醒", en: "Never remind me" },
  "update.checking": { zh: "正在检查更新…", en: "Checking for updates…" },
  "update.upToDateTitle": { zh: "暂无更新", en: "You're up to date" },
  "update.upToDateBody": { zh: "已是最新版本 {0}", en: "You are on the latest version ({0})" },
  "update.downloading": { zh: "正在下载更新…", en: "Downloading update…" },
  "update.progressMb": { zh: "已下载 {0} / {1} MB", en: "{0} of {1} MB downloaded" },
  "update.progressBare": { zh: "已下载 {0} MB", en: "{0} MB downloaded" },
  "update.cancel": { zh: "取消", en: "Cancel" },
  "update.downloadComplete": { zh: "下载完成", en: "Download complete" },
  "update.readyBody": {
    zh: "{0} 的安装包已就绪，点击下方按钮将退出应用并自动安装。",
    en: "The installer for {0} is ready — installing exits the app and runs automatically.",
  },
  "update.installNow": { zh: "立即安装并重启", en: "Install and restart" },
  "update.installingTitle": { zh: "正在安装", en: "Installing" },
  "update.installing": {
    zh: "正在退出并安装，完成后应用将自动重启…",
    en: "Exiting to install — the app restarts automatically when done…",
  },
  "update.failedTitle": { zh: "更新失败", en: "Update failed" },
  "update.failedGeneric": {
    zh: "出了点问题，请稍后重试或前往发布页手动下载。",
    en: "Something went wrong — try again later or download manually from the releases page.",
  },
  "update.retry": { zh: "重试", en: "Retry" },
  "update.close": { zh: "关闭", en: "Close" },

  // Fill tool status (usePaintTool)
  "fill.done": { zh: "已填充分区 {0}（{1} 个面）颜色 {2}", en: "Filled region {0} ({1} faces) with colour {2}" },
  "fill.failed": { zh: "分区填充失败：{0}", en: "Region fill failed: {0}" },
  "paint.pickedColor": { zh: "已拾取颜色：rgb({0},{1},{2})", en: "Picked color: rgb({0},{1},{2})" },
  "paint.error": { zh: "涂色出错：{0}", en: "Paint error: {0}" },

  // SegmentsPanel command feedback (useTauriCommand / SegmentsPanel)
  "cmd.reset.working": { zh: "正在重置分区…", en: "Resetting regions…" },
  "cmd.reset.done": { zh: "已重置：分区与上色均已清除", en: "Reset: regions and paint cleared" },
  "cmd.reset.failed": { zh: "重置失败：{0}", en: "Reset failed: {0}" },
  "cmd.rename.failed": { zh: "重命名失败：{0}", en: "Rename failed: {0}" },
  "cmd.merge.failed": { zh: "合并失败：{0}", en: "Merge failed: {0}" },
  "cmd.split.failed": { zh: "拆分失败：{0}", en: "Split failed: {0}" },
  "cmd.manual.done": { zh: "手动分区完成（共 {0} 个区域）", en: "Manual partition done ({0} regions total)" },
  "cmd.manual.toast": { zh: "添加分区成功", en: "Region added" },
  "cmd.manual.failed": { zh: "手动分区失败：{0}", en: "Manual partition failed: {0}" },
  "cmd.finalizeSegment.done": { zh: "已创建分区（共 {0} 个区域）", en: "Segment created ({0} regions total)" },
  "cmd.resegment.working": { zh: "区域内再分区中…", en: "Re-segmenting region…" },
  "cmd.resegment.done": { zh: "再分区完成：{0} 个区域", en: "Re-segment done: {0} regions" },
  "cmd.resegment.failed": { zh: "再分区失败：{0}", en: "Re-segment failed: {0}" },
  "cmd.seed.working": { zh: "种子生长分区中…", en: "Growing seed partition…" },
  "cmd.seed.failed": { zh: "种子分区失败：{0}", en: "Seed partition failed: {0}" },
  "cmd.suggest.working": { zh: "正在推荐种子点位…", en: "Suggesting seed points…" },
  "cmd.suggest.done": { zh: "已推荐 {0} 个候选种子（点击接受）", en: "Suggested {0} candidate seeds (click to accept)" },
  "cmd.suggest.failed": { zh: "推荐种子失败：{0}", en: "Seed suggestion failed: {0}" },
  "cmd.planar.working": { zh: "正在检测连续平面区域…", en: "Detecting planar regions…" },
  "cmd.planar.done": { zh: "已检测 {0} 个连续平面区域", en: "Detected {0} planar regions" },
  "cmd.planar.failed": { zh: "平面检测失败：{0}", en: "Planar detect failed: {0}" },
  "cmd.multiview.working": { zh: "正在多视角(3→2→3)检测区域…", en: "Detecting regions via MultiView (3→2→3)…" },
  "cmd.multiview.done": { zh: "已检测 {0} 个多视角区域", en: "Detected {0} MultiView regions" },
  "cmd.multiview.failed": { zh: "多视角检测失败：{0}", en: "MultiView detect failed: {0}" },
  "cmd.crossSection.working": { zh: "正在截面(射线)检测特征…", en: "Detecting features via cross-section (rays)…" },
  "cmd.crossSection.done": { zh: "已检测 {0} 个截面特征", en: "Detected {0} cross-section features" },
  "cmd.crossSection.failed": { zh: "截面检测失败：{0}", en: "Cross-section detect failed: {0}" },
  "cmd.eye.working": { zh: "正在语义识别眼睛区域…", en: "Detecting eye regions…" },
  "cmd.eye.done": { zh: "已识别 {0} 个眼睛子区域 ({1})", en: "Detected {0} eye sub-regions ({1})" },
  "cmd.eye.failed": { zh: "眼睛识别失败：{0}", en: "Eye detect failed: {0}" },
  "cmd.eyeAuto.working": { zh: "正在自动识别眼睛…", en: "Auto-detecting eyes…" },
  "cmd.eyeAuto.done": { zh: "自动识别到 {0} 个眼睛子区域 ({1})", en: "Auto-detected {0} eye sub-regions ({1})" },
  "cmd.eyeAuto.failed": { zh: "自动眼睛识别失败：{0}", en: "Auto eye detect failed: {0}" },
  "cmd.fuse.working": { zh: "融合生成分区中…", en: "Fusing partition…" },
  "cmd.fuse.done": { zh: "融合分区完成：{0} 个区域", en: "Fuse done: {0} regions" },
  "cmd.fuse.failed": { zh: "融合分区失败：{0}", en: "Fuse failed: {0}" },

  // SeedPanel status details (fuse summary / grow prep)
  "seed.fuseDetail": { zh: "🧩 融合完成：{0} 区（通道 平面{1}/多视角{2}/折痕{3}/眼{4}, 边 {5}/{6} 切, 区大小 min={7} med={8} max={9}）", en: "🧩 Fuse done: {0} regions (channels planar {1}/multiview {2}/dihedral {3}/eye {4}, edges {5}/{6} cut, size min={7} med={8} max={9})" },
  "seed.resetAllDone": { zh: "✅ 已清空分区（恢复导入时的干净状态）", en: "✅ Regions cleared (back to the freshly-imported clean state)" },
  "seed.growPrep": { zh: "🌱 准备生长（{0} 个种子（手 {1} + 推 {2} + 眼 {3}），barrier={4}°，optimizer={5}）…", en: "🌱 Growing ({0} seeds (manual {1} + suggested {2} + eye {3}), barrier={4}°, optimizer={5})…" },
  "seed.growFailedPanel": { zh: "🌱 种子分区失败：{0}", en: "🌱 Seed partition failed: {0}" },

  // Backend error messages (fuse.rs / seeded.rs / resegment.rs emit stable
  // English error strings; these keys localise them for the status bar).
  "err.meshNoFaces": { zh: "mesh 没有面", en: "mesh has no faces" },
  "err.projectVersion": { zh: "工程格式版本不受支持，请更新 ColorYourModel", en: "Project format version not supported — please update ColorYourModel" },
  "err.projectCorrupt": { zh: "工程文件损坏或不是有效的 .cym 文件", en: "Project file is corrupted or not a valid .cym file" },
  "err.noRegions": { zh: "所有算法均未检测到区域", en: "all detectors returned no regions" },
  "err.needSeed": { zh: "至少需要一个种子点", en: "at least one seed required" },
  "err.seedNoSnap": { zh: "种子点无法吸附到面", en: "seed did not snap to a face" },
  "err.noIncidentFace": { zh: "种子顶点没有相邻面", en: "seed vertex has no incident face" },
  "err.resegmentNoSplit": { zh: "所选算法在该区域内未产生进一步划分（整块仍为一个区域）", en: "the algorithm produced no further split (the region stays whole)" },

  // Algorithm display names for the SegmentsPanel resegment picker.
  "segmentPanel.algo.curvatureKMeans": { zh: "曲率 K-Means（特征感知）", en: "Curvature K-Means (feature-aware)" },
  "segmentPanel.algo.shapeDiameter": { zh: "形态直径 SDF（语义零件）", en: "Shape Diameter SDF (semantic parts)" },
  "segmentPanel.algo.dihedral": { zh: "二面角（法线夹角）", en: "Dihedral angle (normal angle)" },
  "segmentPanel.algo.sdfGraphCut": { zh: "SDF 图割（GMM 全局优化）", en: "SDF Graph-Cut (GMM + global opt)" },
  "segmentPanel.algo.concavity": { zh: "凹度场（沿凹缝切分）", en: "Concavity-Aware Fields (seams)" },
  "segmentPanel.algo.convexDecomposition": { zh: "凸分解 V-HACD（关节处切分）", en: "Convex Decomposition V-HACD (cut at joints)" },
  "segmentPanel.algo.curveSkeleton": { zh: "曲线骨架（肢体级合并）", en: "Curve Skeleton (limb-level merge)" },
  "segmentPanel.algo.fhGraph": { zh: "FH 图分割（自适应粒度）", en: "FH Graph Segmentation (adaptive granularity)" },

  // Segmentation progress stages (iteration 40). The canonical stage keys
  // are emitted by the backend; this map translates them into the human
  // label that shows alongside "Stage X/Y" in the ProgressBar overlay.
  "segStage.header": { zh: "阶段", en: "Stage" },
  "segStage.dihedral.edges": { zh: "测量面间夹角", en: "Measuring face angles" },
  "segStage.dihedral.regions": { zh: "聚合连通区域", en: "Grouping connected regions" },
  "segStage.dihedral.merge": { zh: "归并相似区域", en: "Merging similar regions" },
  "segStage.dihedral.finalize": { zh: "清理碎片区域", en: "Cleaning up small regions" },
  "segStage.fuse.planar": { zh: "检测平面区域", en: "Detecting planar regions" },
  "segStage.fuse.multiview": { zh: "多视角证据渲染", en: "Rendering multi-view evidence" },
  "segStage.fuse.vote": { zh: "边级投票融合分区", en: "Fusing partition by edge vote" },
  "segStage.manual.snap": { zh: "套索：吸附路径点", en: "Lasso: snapping points" },
  "segStage.manual.loop": { zh: "套索：沿表面补全边界", en: "Lasso: completing boundary on surface" },
  "segStage.manual.bfs": { zh: "套索：圈选区域", en: "Lasso: enclosing region" },
  "segStage.manual.smooth": { zh: "套索：平滑边界", en: "Lasso: smoothing boundary" },
  "segStage.manual.commit": { zh: "套索：写入分区", en: "Lasso: committing region" },
  "segStage.sdf.sample": { zh: "采样厚度 (SDF)", en: "Sampling thickness (SDF)" },
  "segStage.sdf.cluster": { zh: "聚类并归并", en: "Clustering & merging" },
  "segStage.sdf.split": { zh: "连通性拆分", en: "Connectivity split" },
  "segStage.curv.features": { zh: "计算特征", en: "Computing features" },
  "segStage.curv.kmeans": { zh: "K-Means 聚类", en: "K-Means clustering" },
  "segStage.curv.refine": { zh: "拆分与轮廓归并", en: "Splitting & contour merge" },
  "segStage.sdf.gmm": { zh: "拟合混合高斯 (GMM)", en: "Fitting Gaussian mixture" },
  "segStage.sdf.graphcut": { zh: "图割全局优化", en: "Graph-cut optimisation" },
  "segStage.sdf.connectivity": { zh: "连通性拆分", en: "Connectivity split" },
  "segStage.concav.topology": { zh: "重建顶点拓扑", en: "Building vertex topology" },
  "segStage.concav.laplacian": { zh: "构造凹度感知矩阵", en: "Assembling concavity matrix" },
  "segStage.concav.field": { zh: "求解标量场", en: "Solving scalar field" },
  "segStage.concav.refine": { zh: "拆分与清理", en: "Splitting & cleanup" },
  "segStage.vhacd.decompose": { zh: "V-HACD 凸分解", en: "V-HACD convex decomposition" },
  "segStage.vhacd.assign": { zh: "按质心分配面到凸块", en: "Assigning faces to hulls" },
  "segStage.skeleton.decompose": { zh: "V-HACD 凸分解", en: "V-HACD convex decomposition" },
  "segStage.skeleton.limbs": { zh: "按关节合并成肢体", en: "Merging chains into limbs" },
  "segStage.fh.significance": { zh: "计算显著性场", en: "Computing significance field" },
  "segStage.fh.edges": { zh: "构造边权图", en: "Building edge-weight graph" },
  "segStage.fh.merge": { zh: "自适应合并区域", en: "Adaptive region merge" },
  "segStage.fh.compact": { zh: "压缩标签", en: "Compacting labels" },
  "segStage.fh.cleanup": { zh: "清理碎片区域", en: "Cleaning up small regions" },
  "segStage.fh.finalize": { zh: "完成", en: "Finalizing" },
  "segStage.finalize.merge": { zh: "清理碎片区域", en: "Cleaning up small regions" },
  "segStage.done": { zh: "完成", en: "Done" },
  "segStage.working": { zh: "处理中", en: "Working" },
};

// ─── Pure translate ───────────────────────────────────────────────
/** Read-only view of the dictionary for tests (completeness / placeholder
 *  parity). Runtime code goes through translate()/translateError(). */
export const I18N_DICT: Readonly<Record<string, Readonly<Record<Lang, string>>>> = dict;

/** Translate `key` for `lang` with positional {0}/{1}/… replacement. */
export function translate(key: string, lang: Lang, ...args: (string | number)[]): string {
  const entry = dict[key];
  if (!entry) return key;
  let text = entry[lang] ?? entry.en ?? key;
  args.forEach((arg, i) => {
    text = text.replace(`{${i}}`, String(arg));
  });
  return text;
}

// ─── Backend error localisation ───────────────────────────────────
// The Rust backend rejects invokes with stable *English* error strings
// (src-tauri segment/fuse/seeded/resegment). Each [needle, key] pair maps
// one of those strings onto a dict entry so the status bar stays fully
// localised; unknown errors pass through untouched.
const backendErrors: Array<[needle: string, key: string]> = [
  ["mesh has no faces", "err.meshNoFaces"],
  ["all detectors returned no regions", "err.noRegions"],
  ["at least one seed required", "err.needSeed"],
  ["seed did not snap to a face", "err.seedNoSnap"],
  ["has no incident face", "err.noIncidentFace"],
  ["no further subdivision", "err.resegmentNoSplit"],
  ["unsupported format version", "err.projectVersion"],
  ["project: model.bin size mismatch", "err.projectCorrupt"],
  ["project: model.bin bad magic", "err.projectCorrupt"],
  ["project: face index out of range", "err.projectCorrupt"],
  ["project: missing entry", "err.projectCorrupt"],
  ["project: parse project.json", "err.projectCorrupt"],
];

/** Localise a backend error string if it matches a known message. */
export function translateError(raw: string, lang: Lang): string {
  for (const [needle, key] of backendErrors) {
    if (raw.includes(needle)) return translate(key, lang);
  }
  return raw;
}
