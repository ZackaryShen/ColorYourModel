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
  "segments.empty": { zh: "暂无分区，请先导入模型", en: "No segments yet. Import a model first." },

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
