import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { useAppStore } from "../../store/appStore";
import { PaintTool } from "../../types/mesh";
import { useTauriCommand } from "../../hooks/useTauriCommand";
import { useUndoRedo } from "../../hooks/useHistory";
import { log } from "../../utils/logger";
import { useT } from "../../i18n";
import { useProjectSave } from "../../hooks/useProjectSave";
import { isGradientTool } from "../../utils/gradientPaint";
import { hintKeyForTool } from "../../utils/controlsHint";

const TOOL_KEYS: { tool: PaintTool; icon: string; i18nKey: string }[] = [
  { tool: PaintTool.View, icon: "🖐️", i18nKey: "tool.view" },
  { tool: PaintTool.Fill, icon: "🪣", i18nKey: "tool.fill" },
  { tool: PaintTool.Brush, icon: "🖌️", i18nKey: "tool.brush" },
  { tool: PaintTool.Spray, icon: "💨", i18nKey: "tool.spray" },
  { tool: PaintTool.SmartBrush, icon: "🎯", i18nKey: "tool.smart" },
  { tool: PaintTool.Eyedropper, icon: "💧", i18nKey: "tool.eyedropper" },
  { tool: PaintTool.Eraser, icon: "🧹", i18nKey: "tool.eraser" },
  { tool: PaintTool.Segment, icon: "✂️", i18nKey: "tool.segment" },
  { tool: PaintTool.Lasso, icon: "📍", i18nKey: "tool.lasso" },
  { tool: PaintTool.Seed, icon: "🌱", i18nKey: "tool.seed" },
];

export function Toolbar({ onImport }: { onImport: () => Promise<void> }) {
  const t = useT();
  const activeTool = useAppStore((s) => s.activeTool);
  const setActiveTool = useAppStore((s) => s.setActiveTool);
  const isLoaded = useAppStore((s) => s.isLoaded);
  const isLoading = useAppStore((s) => s.isLoading);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const setImportProgress = useAppStore((s) => s.setImportProgress);
  // Segment-progress listener writes to a separate slice so the ProgressBar
  // can render the canonical "Stage X/Y" plan instead of the raw loader stage
  // string. `loadingKind` lets the shared overlay pick which slice to read.
  const setSegmentProgress = useAppStore((s) => s.setSegmentProgress);
  const setLoadingKind = useAppStore((s) => s.setLoadingKind);
  const gradientEnabled = useAppStore(s => s.gradientEnabled && !s.meshData?.texturePainting);
  const brushRadius = useAppStore((s) => s.brushRadius);
  const brushStrength = useAppStore((s) => s.brushStrength);
  const { undo, redo, historyState } = useTauriCommand();
  const { undo: doUndo, redo: doRedo, canUndo, canRedo } = useUndoRedo({ undo, redo, historyState });
  const historyPanelOpen = useAppStore((s) => s.historyPanelOpen);
  const setHistoryPanelOpen = useAppStore((s) => s.setHistoryPanelOpen);
  const { saveProject } = useProjectSave();
  const saveBusy = useAppStore((s) => s.saveBusy);
  const importBusy = useAppStore((s) => s.importBusy);
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

  return (
    <div style={styles.container}>
      <div style={styles.section}>
        <button onClick={onImport} disabled={isLoading || importBusy || saveBusy} className="cym-btn" style={styles.button} title={t("toolbar.import")}>
          {isLoading ? "..." : "📂"}
        </button>
        <button
          onClick={() => void saveProject()}
          disabled={!isLoaded || isLoading || importBusy || saveBusy}
          className="cym-btn"
          style={styles.button}
          title={t("menu.save")}
        >
          💾
        </button>
      </div>

      <div style={styles.divider} />

      <div style={styles.section}>
        {TOOL_KEYS.map((item) => {
          // Build dynamic tooltip with current parameters for brush-type tools
          const gradient = gradientEnabled && isGradientTool(item.tool);
          let tip = gradient ? t(hintKeyForTool(item.tool, true)) : t(item.i18nKey);
          if (gradient && item.tool === PaintTool.Brush) {
            tip += ` [${t("brush.radius")}: ${brushRadius}mm]`;
          } else if (
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
        })}
      </div>

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
        {/* Issue #22: the history panel is the visual front-end of the same
            timeline these two buttons drive, so its toggle lives next door. */}
        <button
          onClick={() => setHistoryPanelOpen(!historyPanelOpen)}
          className="cym-btn"
          style={{
            ...styles.toolButton,
            ...(historyPanelOpen ? styles.toolActive : {}),
          }}
          title={t("toolbar.historyPanel")}
        >
          🕘
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
};
