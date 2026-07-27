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
  "lasso.hint": { zh: "套索模式：点击顶点选点，回到起点闭合区域（Esc 取消）", en: "Lasso: click vertices; click the start point to close (Esc to cancel)" },
  "lasso.need3": { zh: "至少需要 3 个点才能闭合区域", en: "Need at least 3 points to close a region" },
  "lasso.cancelled": { zh: "已取消当前套索", en: "Lasso cancelled" },
  "toolbar.importComplete": { zh: "导入完成", en: "Import complete" },
  "toolbar.importFailed": { zh: "导入失败", en: "Import failed" },
  "toolbar.startImport": { zh: "开始导入…", en: "Starting import…" },

  // Toolbar — tool tooltips (differentiated descriptions)
  "tool.fill": { zh: "🪣 填充 — 点击任意面，一键填满整个分区", en: "🪣 Fill — click any face to fill entire connected region" },
  "tool.brush": { zh: "🖌️ 画笔 — 平滑连续涂色，边缘渐变过渡", en: "🖌️ Brush — smooth continuous paint with edge falloff" },
  "tool.spray": { zh: "💨 喷罐 — 随机散点喷涂，模拟真实喷漆颗粒感", en: "💨 Spray — random scatter dots, simulates real spray-can texture" },
  "tool.smart": { zh: "🎯 智能笔 — 自动识别分区边界，不会涂出区域", en: "🎯 Smart — segment-aware, stays within region boundary" },
  "tool.eyedropper": { zh: "💧 吸管 — 从模型表面拾取颜色", en: "💧 Eyedropper — pick color from model surface" },
  "tool.eraser": { zh: "🧹 橡皮 — 擦除颜色，恢复为白色", en: "🧹 Eraser — restore to original white" },
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

  // ColorPanel
  "color.title": { zh: "颜色", en: "Color" },
  "color.amsPalette": { zh: "AMS 调色板", en: "AMS Palette" },

  // SegmentsPanel
  "segments.title": { zh: "分区", en: "Regions" },
  "segments.empty": { zh: "暂无分区，请先导入模型", en: "No segments yet. Import a model first." },

  // Viewport — ControlsHelp
  "controls.leftPaint": { zh: "🖱️ 左键: 涂色", en: "🖱️ Left: Paint" },
  "controls.rightRotate": { zh: "🔄 右键+拖: 旋转", en: "🔄 Right+Drag: Rotate" },
  "controls.middlePan": { zh: "✋ 中键+拖 / Space+左键: 平移", en: "✋ Middle+Drag / Space+Left: Pan" },
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
  "status.tip": { zh: "右键: 旋转 | 中键: 平移 | 滚轮: 缩放", en: "Right-click: Rotate | Middle: Pan | Scroll: Zoom" },
  "status.ready": { zh: "就绪", en: "Ready" },
  "status.loaded": { zh: "已加载 {0} 个面", en: "Loaded {0} faces" },

  // Language
  "lang.switch": { zh: "EN", en: "中" },
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
