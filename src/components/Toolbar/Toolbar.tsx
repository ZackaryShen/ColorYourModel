import { listen } from "@tauri-apps/api/event";
import { Fragment, useEffect, useState } from "react";
import { useAppStore } from "../../store/appStore";
import { PaintTool } from "../../types/mesh";
import { useTauriCommand } from "../../hooks/useTauriCommand";
import { useUndoRedo } from "../../hooks/useHistory";
import { useImportStl } from "../../hooks/useImportStl";
import { log } from "../../utils/logger";
import { useT } from "../../i18n";
import { ExportDialog } from "../ExportDialog/ExportDialog";

const TOOL_KEYS: { tool: PaintTool; icon: string; i18nKey: string }[] = [
  { tool: PaintTool.View, icon: "🖐️", i18nKey: "tool.view" },
  { tool: PaintTool.Fill, icon: "🪣", i18nKey: "tool.fill" },
  { tool: PaintTool.Brush, icon: "🖌️", i18nKey: "tool.brush" },
  { tool: PaintTool.Gradient, icon: "🌈", i18nKey: "tool.gradient" },
  { tool: PaintTool.Spray, icon: "💨", i18nKey: "tool.spray" },
  { tool: PaintTool.SmartBrush, icon: "🎯", i18nKey: "tool.smart" },
  { tool: PaintTool.Eyedropper, icon: "💧", i18nKey: "tool.eyedropper" },
  { tool: PaintTool.Eraser, icon: "🧹", i18nKey: "tool.eraser" },
  { tool: PaintTool.Segment, icon: "✂️", i18nKey: "tool.segment" },
  { tool: PaintTool.Lasso, icon: "📍", i18nKey: "tool.lasso" },
  { tool: PaintTool.Seed, icon: "🌱", i18nKey: "tool.seed" },
];

// 0.2.0-P1 (req #6): paint and segment tools are grouped and collapsible.
// View stays always-visible above the groups; undo/redo stays at the bottom.
// Group headers are chevron-only (the 56px bar has no room for text labels —
// adversarial review M4); the group identity travels in the tooltip.
const TOOL_GROUPS: { id: "paint" | "segment"; labelKey: string; tools: typeof TOOL_KEYS }[] = [
  { id: "paint", labelKey: "toolbar.groupPaint", tools: TOOL_KEYS.slice(1, 8) },
  { id: "segment", labelKey: "toolbar.groupSegment", tools: TOOL_KEYS.slice(7, 10) },
];

export function Toolbar() {
  const t = useT();
  const activeTool = useAppStore((s) => s.activeTool);
  const setActiveTool = useAppStore((s) => s.setActiveTool);
  const isLoaded = useAppStore((s) => s.isLoaded);
  const isLoading = useAppStore((s) => s.isLoading);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const setLoading = useAppStore((s) => s.setLoading);
  const setImportProgress = useAppStore((s) => s.setImportProgress);
  // Segment-progress listener writes to a separate slice so the ProgressBar
  // can render the canonical "Stage X/Y" plan instead of the raw loader stage
  // string. `loadingKind` lets the shared overlay pick which slice to read.
  const setSegmentProgress = useAppStore((s) => s.setSegmentProgress);
  const setLoadingKind = useAppStore((s) => s.setLoadingKind);
  const brushRadius = useAppStore((s) => s.brushRadius);
  const brushStrength = useAppStore((s) => s.brushStrength);
  const { loadModel, undo, redo, historyState } = useTauriCommand();
  const { undo: doUndo, redo: doRedo, canUndo, canRedo } = useUndoRedo({ undo, redo, historyState });
  const handleImport = useImportStl();
  const [exportDialogOpen, setExportDialogOpen] = useState(false);
  // Register progress listeners at mount time (avoids race condition + leak)
  useEffect(() => {
    const unlistenImport = listen<{ progress: number; stage: string }>(
      "import-progress",
      (e) => {
        log.debug("Toolbar", "import-progress", e.payload);
        setImportProgress(e.payload.progress, e.payload.stage);
        setLoadingKind("import");
      }
    );
    const unlistenSegment = listen<{ progress: number; stage: string }>(
      "segment-progress",
      (e) => {
        log.debug("Toolbar", "segment-progress", e.payload);
        setSegmentProgress(e.payload.progress, e.payload.stage);
        setLoadingKind("segment");
      }
    );
    return () => {
      unlistenImport.then((fn) => fn());
      unlistenSegment.then((fn) => fn());
    };
  }, [setImportProgress, setSegmentProgress, setLoadingKind]);

  // handleImport comes from the shared useImportStl hook (also used by the
  // File menu): file picker → import progress → loadModel → status messages.

  const handleExport = () => {
    setExportDialogOpen(true);
  };

  // Collapse state is intentionally session-local (not persisted): the bar is
  // small and default-expanded is the right first-run experience.
  const [collapsed, setCollapsed] = useState<{ paint: boolean; segment: boolean }>({
    paint: false,
    segment: false,
  });
  const toggleGroup = (id: "paint" | "segment") => {
    setCollapsed((prev) => {
      if (prev[id]) return { ...prev, [id]: false };
      // Collapsing a group that owns the active tool would hide the active
      // tool's button while activeTool stays on it — the user would lose all
      // visible indication of the current mode. Refuse instead (review M4).
      const group = TOOL_GROUPS.find((g) => g.id === id)!;
      if (group.tools.some((tk) => tk.tool === activeTool)) return prev;
      return { ...prev, [id]: true };
    });
  };

  const renderToolButton = (item: { tool: PaintTool; icon: string; i18nKey: string }) => {
    // Build dynamic tooltip with current parameters for brush-type tools
    let tip = t(item.i18nKey);
    if (
      item.tool === PaintTool.Brush ||
      item.tool === PaintTool.Spray ||
      item.tool === PaintTool.SmartBrush
    ) {
      tip += ` [${t("brush.radius")}: ${brushRadius}mm, ${t("brush.strength")}: ${(brushStrength * 100).toFixed(0)}%]`;
    }
    return (
      <button
        key={item.tool}
        onClick={(e) => {
          e.currentTarget.blur(); // drop focus so the just-clicked button
          // doesn't keep a focus-ring + hover filter together with the
          // newly-active tool's border (looked like "two tools active").
          log.info("Toolbar", "tool click", { tool: item.tool, from: activeTool });
          setActiveTool(item.tool);
        }}
        className="cym-btn"
        style={{
          ...styles.toolButton,
          ...(activeTool === item.tool ? styles.toolActive : {}),
        }}
        title={tip}
      >
        {item.icon}
      </button>
    );
  };

  return (
    <div style={styles.container}>
      {exportDialogOpen && <ExportDialog onClose={() => setExportDialogOpen(false)} />}
      <div style={styles.section}>
        <button onClick={handleImport} disabled={isLoading} className="cym-btn" style={styles.button} title={t("toolbar.import")}>
          {isLoading ? "..." : "📂"}
        </button>
        <button
          onClick={handleExport}
          disabled={!isLoaded}
          className="cym-btn"
          style={styles.button}
          title={t("toolbar.export")}
        >
          💾
        </button>
      </div>

      <div style={styles.divider} />

      <div style={styles.section}>{renderToolButton(TOOL_KEYS[0])}</div>

      {TOOL_GROUPS.map((group, gi) => (
        <Fragment key={group.id}>
          {gi > 0 && <div style={styles.divider} />}
          <div style={styles.section}>
            <button
              className="cym-btn"
              style={styles.groupChevron}
              onClick={() => toggleGroup(group.id)}
              title={`${t("toolbar.groupCollapse")} — ${t(group.labelKey)}`}
              aria-label={`${t(group.labelKey)} — ${t("toolbar.groupCollapse")}`}
            >
              {collapsed[group.id] ? "▸" : "▾"}
            </button>
            {!collapsed[group.id] && group.tools.map(renderToolButton)}
          </div>
        </Fragment>
      ))}

      <div style={styles.divider} />

      <div style={styles.section}>
        <button
          onClick={doUndo}
          disabled={!canUndo}
          className="cym-btn"
          style={styles.button}
          title={t("toolbar.undo")}
        >
          ↶
        </button>
        <button
          onClick={doRedo}
          disabled={!canRedo}
          className="cym-btn"
          style={styles.button}
          title={t("toolbar.redo")}
        >
          ↷
        </button>
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  container: {
    display: "flex",
    flexDirection: "column",
    gap: 4,
    padding: 8,
    background: "var(--bg-panel, #2d2d2d)",
    borderRadius: 8,
    minWidth: 56,
    alignItems: "center",
  },
  section: {
    display: "flex",
    flexDirection: "column",
    gap: 4,
  },
  button: {
    // Same footprint as the tool buttons: import/export used to render
    // smaller (6px/8px padding vs the 40×40 tool squares), which read as a
    // second, mismatched toolbar row (GUI audit round 2, B13).
    width: 40,
    height: 40,
    border: "none",
    borderRadius: 8,
    background: "var(--bg-hover, #444444)",
    color: "var(--text-1, #eeeeee)",
    cursor: "pointer",
    fontSize: 18,
    whiteSpace: "nowrap" as const,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
  },
  toolButton: {
    width: 40,
    height: 40,
    border: "2px solid transparent",
    borderRadius: 8,
    background: "var(--bg-elevated, #3a3a3a)",
    color: "var(--text-1, #eeeeee)",
    cursor: "pointer",
    fontSize: 18,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
  },
  toolActive: {
    borderColor: "var(--accent, #4a9eff)",
    background: "var(--bg-active, #3a5a7a)",
    // Inset shadow + outer glow so the active state stays clearly distinct
    // from the hover state (a 1.15× brightness filter on a neighbour button
    // could otherwise read as "two tools active at once").
    boxShadow:
      "inset 0 0 0 2px var(--accent, #4a9eff), 0 0 0 2px var(--accent, #4a9eff)",
  },
  divider: {
    width: "80%",
    height: 1,
    background: "var(--border, #555555)",
    margin: "4px 0",
  },
  groupChevron: {
    width: 40,
    height: 14,
    border: "none",
    background: "transparent",
    color: "var(--text-2, #aaaaaa)",
    cursor: "pointer",
    fontSize: 10,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    padding: 0,
  },
};
