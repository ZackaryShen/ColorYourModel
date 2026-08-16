/**
 * Lightweight i18n system — Chinese (default) / English.
 * Usage: const t = useT();  then  t("toolbar.import")
 */
import { useAppStore } from "./store/appStore";

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
  "toolbar.importFailed": { zh: "导入失败", en: "Import failed" },
  "toolbar.startImport": { zh: "开始导入…", en: "Starting import…" },
  "toolbar.segmentFailed": { zh: "模型已加载，但自动分区失败——填充将降级为画笔", en: "Model loaded, but auto-segmentation failed — Fill degrades to brush" },

  // Toolbar — tool tooltips (differentiated descriptions)
  "tool.view": { zh: "🖐️ 查看/导航 — 左键旋转、右键平移、中键缩放；选工具后按住 Alt 也可旋转", en: "🖐️ View / Navigate — left rotate, right pan, middle zoom; hold Alt to rotate while a tool is active" },
  "tool.fill": { zh: "🪣 填充 — 点击任意面，一键填满整个分区", en: "🪣 Fill — click any face to fill entire connected region" },
  "tool.brush": { zh: "🖌️ 画笔 — 平滑连续涂色，边缘渐变过渡", en: "🖌️ Brush — smooth continuous paint with edge falloff" },
  "tool.spray": { zh: "💨 喷罐 — 随机散点喷涂，模拟真实喷漆颗粒感", en: "💨 Spray — random scatter dots, simulates real spray-can texture" },
  "tool.smart": { zh: "🎯 智能笔 — 自动识别分区边界，不会涂出区域", en: "🎯 Smart — segment-aware, stays within region boundary" },
  "tool.eyedropper": { zh: "💧 吸管 — 从模型表面拾取颜色", en: "💧 Eyedropper — pick color from model surface" },
  "tool.eraser": { zh: "🧹 橡皮 — 擦除颜色，恢复为默认底色", en: "🧹 Eraser — restore the default base color" },
  "tool.segment": { zh: "✂️ 分区画笔 — 拖拽涂选面，松开鼠标创建分区", en: "✂️ Segment Brush — drag to paint faces, release to create region" },
  "tool.lasso": { zh: "📍 选点套索 — 依次点选顶点围合区域，点击起点闭合", en: "📍 Lasso — click vertices to outline a region; click the start point to close" },
  "tool.seed": { zh: "🌱 种子分区 — 在模型上点选若干种子，算法按几何智能长成区域", en: "🌱 Seed — drop seed points; the algorithm grows each into a region by geometry" },
  "seed.hint": { zh: "种子分区：在模型上点击放置种子（每点一个区域），调节屏障角度后点「生长」。未点的区域由最近种子兜底。Backspace/Esc 清空。", en: "Seed: click to place seeds (one region each), tune the barrier angle, then Grow. Un-seeded patches fall back to the nearest seed. Backspace/Esc clears." },
  "seed.barrier": { zh: "屏障角度 (°)", en: "Barrier angle (°)" },
  "seed.barrierHint": { zh: "大于此二面角的棱成为硬边界，区域不会越过折缝", en: "Edges above this dihedral angle are hard boundaries the regions won't cross" },
  "seed.optimizer": { zh: "智能优化（合并微小区域）", en: "Optimize (merge tiny regions)" },
  "seed.grow": { zh: "🌱 生长", en: "🌱 Grow" },
  "seed.clear": { zh: "清空种子", en: "Clear seeds" },
  "seed.count": { zh: "已放置 {0} 个种子", en: "{0} seeds placed" },
  "seed.needOne": { zh: "请至少放置一个种子", en: "Place at least one seed first" },
  "seed.done": { zh: "种子分区完成：{0} 个区域", en: "Seed partition done: {0} regions" },
  "seed.eraseMode": { zh: "擦除模式", en: "Eraser" },
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
  "seed.fuseTitle": { zh: "把 Layer 1 平面 + Layer 3 多视角的区域成员关系按边级多数投票融合成最终分区并落盘（不再依赖种子点）", en: "Fuse Layer 1 planar + Layer 3 MultiView region membership by edge-level majority vote into the final partition (no seed points needed)" },
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

  // BrushSettings
  "brush.title": { zh: "画笔设置", en: "Brush Settings" },
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

  // Viewport — SegmentToggle
  "view.paintView": { zh: "🎨 涂色视图", en: "🎨 Paint View" },
  "view.segmentView": { zh: "🗺️ 分区视图", en: "🗺️ Segment View" },
  "view.switchToPaint": { zh: "切换到涂色视图", en: "Switch to paint view" },
  "view.showSegments": { zh: "显示分区着色", en: "Show segment regions" },

  // Viewport — misc
  "view.loading": { zh: "加载中…", en: "Loading…" },
  "view.emptyState": { zh: "导入 STL 文件开始涂装", en: "Import an STL file to get started" },

  // StatusBar
  "status.tool": { zh: "工具", en: "Tool" },
  "status.faces": { zh: "面数", en: "Faces" },
  "status.tip": { zh: "左键: 旋转(查看) | 右键: 平移 | 中键: 缩放 | 滚轮: 缩放 | Alt+左键: 旋转", en: "Left: Rotate (View) | Right: Pan | Middle: Zoom | Scroll: Zoom | Alt+Left: Rotate" },
  "status.ready": { zh: "就绪", en: "Ready" },
  "status.loaded": { zh: "已加载 {0} 个面", en: "Loaded {0} faces" },

  // Language
  "lang.switch": { zh: "EN", en: "中" },

  // Theme (iteration 20)
  "theme.toggle": { zh: "切换深浅色", en: "Toggle theme" },
  "theme.dark": { zh: "🌙 深色", en: "🌙 Dark" },
  "theme.light": { zh: "☀️ 浅色", en: "☀️ Light" },

  // Export dialog (preset library)
  "export.title": { zh: "导出为 3MF", en: "Export as 3MF" },
  "export.machine": { zh: "厂商 / 机型", en: "Vendor / Machine" },
  "export.nozzle": { zh: "喷嘴直径", en: "Nozzle diameter" },
  "export.process": { zh: "工艺（打印参数）", en: "Process (print profile)" },
  "export.filament": { zh: "耗材（每槽位）", en: "Filament (per slot)" },
  "export.slots": { zh: " 个槽位", en: " slots" },
  "export.target": { zh: "目标切片器", en: "Target slicer" },
  "export.targetSnapmaker": { zh: "Snapmaker Orca", en: "Snapmaker Orca" },
  "export.targetOrca": { zh: "OrcaSlicer（通用）", en: "OrcaSlicer (generic)" },
  "export.cancel": { zh: "取消", en: "Cancel" },
  "export.confirm": { zh: "选择保存位置并导出", en: "Choose location & export" },
  "export.exporting": { zh: "导出中…", en: "Exporting…" },

  // Intelligent segmentation panel
  "segmentPanel.toolbar": { zh: "🤖 智能分区", en: "🤖 Segment" },
  "segmentPanel.title": { zh: "智能分区", en: "Intelligent Segmentation" },
  "segmentPanel.algorithm": { zh: "算法", en: "Algorithm" },
  "segmentPanel.algo.curvatureKMeans": { zh: "曲率 K-Means（特征感知）", en: "Curvature K-Means (feature-aware)" },
  "segmentPanel.algo.shapeDiameter": { zh: "形态直径 SDF（语义零件）", en: "Shape Diameter SDF (semantic parts)" },
  "segmentPanel.algo.dihedral": { zh: "二面角（法线夹角）", en: "Dihedral angle (normal angle)" },
  "segmentPanel.algo.sdfGraphCut": { zh: "SDF 图割（GMM 全局优化）", en: "SDF Graph-Cut (GMM + global opt)" },
  "segmentPanel.algo.concavity": { zh: "凹度场（沿凹缝切分）", en: "Concavity-Aware Fields (seams)" },
  "segmentPanel.algo.convexDecomposition": { zh: "凸分解 V-HACD（关节处切分）", en: "Convex Decomposition V-HACD (cut at joints)" },
  "segmentPanel.algo.curveSkeleton": { zh: "曲线骨架（肢体级合并）", en: "Curve Skeleton (limb-level merge)" },
  "segmentPanel.algo.fhGraph": { zh: "FH 图分割（自适应粒度）", en: "FH Graph Segmentation (adaptive granularity)" },
  "segmentPanel.algoDesc.curvatureKMeans": { zh: "按曲率（弯曲程度）把曲面聚成 k 类。对光滑有机形状尚可，但凹缝不明显时极易过切，碎成几十上百个碎片区，仅适合先快速预览。", en: "Clusters faces by surface curvature. Works on smooth organic shapes but over-splits when seams are weak, leaving dozens of fragments. Use for a quick preview only." },
  "segmentPanel.algoDesc.shapeDiameter": { zh: "按「从表面到另一侧厚度」聚类：粗的躯干与细的四肢自然分开。适合凹缝不明显的动物、手办、角色模型。", en: "Clusters by shape diameter (thickness from a face to the far side), so thick torso vs thin limbs separate naturally. Best for animals, figures, characters with weak seams." },
  "segmentPanel.algoDesc.dihedral": { zh: "按相邻面法线夹角切分，超过阈值的棱成为分区边界。最快、结果最可预期，适合卡扣、法兰、浅浮雕等硬边零件。", en: "Cuts where the angle between adjacent faces exceeds a threshold. Fastest and most predictable; ideal for hard-edged parts like clips, flanges, reliefs." },
  "segmentPanel.algoDesc.sdfGraphCut": { zh: "在形态直径基础上用高斯混合模型 + 图割做全局最优归并，边界更整块平滑。但计算慢，且偏向厚度峰，对人形 / 铠甲容易过切。", en: "Adds a Gaussian-mixture + graph-cut global optimisation on top of SDF for smoother whole parts. Slower, biased to thickness peaks — tends to over-split humanoid/armour models." },
  "segmentPanel.algoDesc.concavity": { zh: "把凹缝（颈窝、腋下、膝窝、胯缝）直接当成分区边界，最贴近人类「头 / 躯干 / 四肢」的部件直觉。最适合人型、铠甲、拼装件；对纯曲面（龙、手办）效果一般。人型已自动细分至 12–20 区，还想切手指 / 关节可调到 20–48。", en: "Treats concavities (neck pit, armpit, knee, crotch seam) as part boundaries — closest to the human intuition of head/torso/limbs. Best for humanoids, armour, assembled kits; weaker on pure surfaces (dragons, figures). Humanoids auto-split into 12–20 regions; raise to 20–48 to separate fingers/joints." },
  "segmentPanel.algoDesc.convexDecomposition": { zh: "用 V-HACD 近似凸分解，在模型最窄的「关节」（颈、腰、腕、踝）处把模型切成近似凸的块。铠甲上的凸脊没有凹度信号，凹度场在此失效，而凸分解恰恰在关节处切——最适合人型、铠甲、拼装件。块较细，可手动合并。", en: "Approximate convex decomposition (V-HACD) cuts the model at its narrowest joints (neck, waist, wrist, ankle) into roughly-convex blocks. Armour ridges carry no concavity signal so the concavity field fails there, but convex decomposition cuts exactly at the joints — best for humanoids, armour, kits. Blocks are fine; merge manually if needed." },
  "segmentPanel.algoDesc.curveSkeleton": { zh: "在凸分解的基础上，把两个关节之间的「链」合并成一个肢体级区域（头 / 躯干 / 四肢各成一块），比纯凸分解更粗、更贴近语义。同一遍 V-HACD 计算，适合想要整体大块上色、再局部细分的场景。", en: "Built on top of convex decomposition: each chain between joints is merged into one limb-level region (head / torso / each limb as a single block), coarser and more semantic than the raw blocks. Same V-HACD pass; good when you want big blocks to paint first, then refine locally." },
  "segmentPanel.algoDesc.fhGraph": { zh: "Felzenszwalb-Huttenlocher 图分割（移植自 SAM3D 末段）。不需要预设分区数 k，只用一个「粒度 scale」：边权低于自适应阈值 Int(C)+scale/|C| 才合并，零件数由几何自动涌现。边权 = 曲率/凹度显著性场之差，复用种子推荐的同一套权重。纯几何、无 ML、无 GPU。scale 越小越细、越大越整块。", en: "Felzenszwalb-Huttenlocher graph segmentation (ported from SAM3D's final stage). Takes a granularity scale instead of a preset k: an edge merges only while its weight is below the adaptive threshold Int(C)+scale/|C|, so the part count emerges from the geometry. Edge weight = the curvature/concavity significance-field difference, reusing the same weights as seed suggestion. Pure-geometric, no ML, no GPU. Smaller scale = finer, larger = coarser." },
  "segmentPanel.angle": { zh: "二面角阈值 (°)", en: "Dihedral threshold (°)" },
  "segmentPanel.clusters": { zh: "聚类数 k", en: "Clusters k" },
  "segmentPanel.clustersAuto": { zh: "0 = 自动估计聚类数", en: "0 = auto-estimate cluster count" },
  "segmentPanel.smoothing": { zh: "法线平滑迭代", en: "Normal smoothing iters" },
  "segmentPanel.useSdf": { zh: "融合形态直径特征", en: "Blend shape-diameter feature" },
  "segmentPanel.useSdfHint": { zh: "勾选后同时考虑厚度，更易区分粗/细零件", en: "Also considers thickness to separate thick/thin parts" },
  "segmentPanel.crease": { zh: "棱角阈值 (°)", en: "Crease threshold (°)" },
  "segmentPanel.creaseHint": { zh: "低于此角度的边界被溶解，零件更整块", en: "Smoother borders below this angle dissolve into whole parts" },
  "segmentPanel.maxHulls": { zh: "凸块数上限", en: "Max convex hulls" },
  "segmentPanel.maxHullsAuto": { zh: "0 = 自动（约 32 块）", en: "0 = auto (~32 blocks)" },
  "segmentPanel.concavityTol": { zh: "凹凸容忍度 (%)", en: "Concavity tolerance (%)" },
  "segmentPanel.concavityTolHint": { zh: "越小块越多、贴合越紧；越大块越少", en: "Lower = more blocks, tighter fit; higher = fewer blocks" },
  "segmentPanel.fhScale": { zh: "粒度 scale", en: "Granularity scale" },
  "segmentPanel.fhScaleHint": { zh: "越小切得越细、分区越多；越大越整块", en: "Smaller = finer / more regions; larger = coarser" },
  "segmentPanel.run": { zh: "运行分区", en: "Run segmentation" },
  "segmentPanel.running": { zh: "分区中…", en: "Segmenting…" },
  "segmentPanel.cancel": { zh: "取消", en: "Cancel" },

  // Segmentation progress stages (iteration 40). The canonical stage keys
  // are emitted by the backend; this map translates them into the human
  // label that shows alongside "Stage X/Y" in the ProgressBar overlay.
  "segStage.header": { zh: "阶段", en: "Stage" },
  "segStage.dihedral.edges": { zh: "测量面间夹角", en: "Measuring face angles" },
  "segStage.dihedral.regions": { zh: "聚合连通区域", en: "Grouping connected regions" },
  "segStage.dihedral.merge": { zh: "归并相似区域", en: "Merging similar regions" },
  "segStage.dihedral.finalize": { zh: "清理碎片区域", en: "Cleaning up small regions" },
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

// ─── Hook: returns translation function ──────────────────────────
export function useT() {
  const lang = useAppStore((s) => s.language);
  return (key: string, ...args: (string | number)[]): string => {
    const entry = dict[key];
    if (!entry) return key;
    let text = entry[lang] ?? entry.en ?? key;
    // Simple positional replacement: {0}, {1}, …
    args.forEach((arg, i) => {
      text = text.replace(`{${i}}`, String(arg));
    });
    return text;
  };
}
