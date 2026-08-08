import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import { useAppStore } from "../../store/appStore";
import { PaintTool } from "../../types/mesh";
import { useTauriCommand } from "../../hooks/useTauriCommand";
import { useUndoRedo } from "../../hooks/useHistory";
import { log } from "../../utils/logger";
import { useT } from "../../i18n";
import { ExportDialog } from "../ExportDialog/ExportDialog";
import { IntelligentSegmentPanel } from "../IntelligentSegmentPanel";
import {
  buildAlgorithm,
  DEFAULT_ALGORITHM_PARAMS,
  type AlgorithmKind,
} from "../../types/segment";

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
  const brushRadius = useAppStore((s) => s.brushRadius);
  const brushStrength = useAppStore((s) => s.brushStrength);
  const { loadModel, autoSegmentV2, undo, redo, historyState } = useTauriCommand();
  const { undo: doUndo, redo: doRedo, canUndo, canRedo } = useUndoRedo({ undo, redo, historyState });
  const [exportDialogOpen, setExportDialogOpen] = useState(false);
  const [segmentPanelOpen, setSegmentPanelOpen] = useState(false);
  // Persisted last segmentation choice: used both on import (so re-import
  // auto-segments with the user's preferred algorithm instead of a hard-coded
  // 30° dihedral) and when the panel is opened.
  const lastAlgorithmParams = useAppStore((s) => s.lastAlgorithmParams);
  const lastSegmentKind = useAppStore((s) => s.lastSegmentKind);

  // Register progress listeners at mount time (avoids race condition + leak)
  useEffect(() => {
    const unlistenImport = listen<{ progress: number; stage: string }>(
      "import-progress",
      (e) => {
        log.debug("Toolbar", "import-progress", e.payload);
        setImportProgress(e.payload.progress, e.payload.stage);
      }
    );
    const unlistenSegment = listen<{ progress: number; stage: string }>(
      "segment-progress",
      (e) => {
        log.debug("Toolbar", "segment-progress", e.payload);
        setImportProgress(e.payload.progress, e.payload.stage);
      }
    );
    return () => {
      unlistenImport.then((fn) => fn());
      unlistenSegment.then((fn) => fn());
    };
  }, [setImportProgress]);

  const handleImport = async () => {
    log.info("Toolbar", "Import button clicked");
    const selected = await open({
      multiple: false,
      filters: [{ name: "3D Models", extensions: ["stl"] }],
    });
    if (!selected) {
      log.debug("Toolbar", "File dialog cancelled");
      return;
    }

    log.info("Toolbar", `File selected: ${selected}`);
    setLoading(true);
    setImportProgress(0, t("toolbar.startImport"));

    try {
      await loadModel(selected);
    } catch (e) {
      log.error("Toolbar", "Import failed", { error: String(e) });
      setStatusMessage(`${t("toolbar.importFailed")}: ${e}`);
      return;
    }

    // Auto-segmentation failure must not be swallowed: the model IS loaded and
    // usable, but with an empty segment list Fill has no partition to target
    // and silently degrades to the brush (REFUTE, docs/06 §2.1 item 3). Say so
    // explicitly instead of letting the user discover "Fill = brush" later.
    try {
      // Use the user's last chosen algorithm (persisted); default to a 30°
      // dihedral which matches the previous hard-coded import behaviour. This
      // removes the magic constant from the call site — the value now flows from
      // the persisted prefs / DEFAULT_ALGORITHM_PARAMS instead of a literal.
      const segKind: AlgorithmKind = lastSegmentKind ?? "dihedral";
      const segParams = lastAlgorithmParams ?? DEFAULT_ALGORITHM_PARAMS;
      await autoSegmentV2(buildAlgorithm(segKind, segParams));
      setImportProgress(1, t("toolbar.importComplete"));
      setStatusMessage(t("toolbar.importComplete"));
    } catch (e) {
      log.error("Toolbar", "Auto-segment failed", { error: String(e) });
      setStatusMessage(`${t("toolbar.segmentFailed")}: ${e}`);
    } finally {
      setLoading(false);
    }
  };

  const handleExport = () => {
    setExportDialogOpen(true);
  };

  return (
    <div style={styles.container}>
      {exportDialogOpen && <ExportDialog onClose={() => setExportDialogOpen(false)} />}
      {segmentPanelOpen && (
        <IntelligentSegmentPanel onClose={() => setSegmentPanelOpen(false)} />
      )}
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

      <div style={styles.section}>
        {TOOL_KEYS.map((item) => {
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
              onClick={() => setActiveTool(item.tool)}
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
      </div>

      <div style={styles.divider} />

      <div style={styles.section}>
        <button
          onClick={() => setSegmentPanelOpen(true)}
          disabled={!isLoaded}
          className="cym-btn"
          style={styles.button}
          title={t("segmentPanel.toolbar")}
        >
          🤖
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
    padding: "6px 8px",
    border: "none",
    borderRadius: 6,
    background: "var(--bg-hover, #444444)",
    color: "var(--text-1, #eeeeee)",
    cursor: "pointer",
    fontSize: 13,
    whiteSpace: "nowrap" as const,
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
  },
  divider: {
    width: "80%",
    height: 1,
    background: "var(--border, #555555)",
    margin: "4px 0",
  },
};
