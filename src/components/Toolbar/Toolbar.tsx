import { open, save } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { useAppStore } from "../../store/appStore";
import { PaintTool } from "../../types/mesh";
import { useTauriCommand } from "../../hooks/useTauriCommand";
import { log } from "../../utils/logger";
import { useT } from "../../i18n";

const TOOL_KEYS: { tool: PaintTool; icon: string; i18nKey: string }[] = [
  { tool: PaintTool.Fill, icon: "🪣", i18nKey: "tool.fill" },
  { tool: PaintTool.Brush, icon: "🖌️", i18nKey: "tool.brush" },
  { tool: PaintTool.Spray, icon: "💨", i18nKey: "tool.spray" },
  { tool: PaintTool.SmartBrush, icon: "🎯", i18nKey: "tool.smart" },
  { tool: PaintTool.Eyedropper, icon: "💧", i18nKey: "tool.eyedropper" },
  { tool: PaintTool.Eraser, icon: "🧹", i18nKey: "tool.eraser" },
  { tool: PaintTool.Segment, icon: "✂️", i18nKey: "tool.segment" },
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
  const { loadModel, autoSegment, export3mf } = useTauriCommand();

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
      await autoSegment(30.0);
      setImportProgress(1, t("toolbar.importComplete"));
      setStatusMessage(t("toolbar.importComplete"));
    } catch (e) {
      log.error("Toolbar", "Import failed", { error: String(e) });
      setStatusMessage(`${t("toolbar.importFailed")}: ${e}`);
    } finally {
      setLoading(false);
    }
  };

  const handleExport = async () => {
    const selected = await save({
      filters: [{ name: "3MF", extensions: ["3mf"] }],
      defaultPath: "model.3mf",
    });
    if (selected) {
      await export3mf(selected);
    }
  };

  const handleSegment = async () => {
    await autoSegment(30.0);
  };

  return (
    <div style={styles.container}>
      <div style={styles.section}>
        <button onClick={handleImport} disabled={isLoading} style={styles.button} title={t("toolbar.import")}>
          {isLoading ? "..." : "📂"}
        </button>
        <button
          onClick={handleExport}
          disabled={!isLoaded}
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
          onClick={handleSegment}
          disabled={!isLoaded}
          style={styles.button}
          title={t("toolbar.reSegment")}
        >
          🔀
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
    background: "#2d2d2d",
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
    background: "#444",
    color: "#eee",
    cursor: "pointer",
    fontSize: 13,
    whiteSpace: "nowrap" as const,
  },
  toolButton: {
    width: 40,
    height: 40,
    border: "2px solid transparent",
    borderRadius: 8,
    background: "#3a3a3a",
    color: "#eee",
    cursor: "pointer",
    fontSize: 18,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
  },
  toolActive: {
    borderColor: "#4a9eff",
    background: "#3a5a7a",
  },
  divider: {
    width: "80%",
    height: 1,
    background: "#555",
    margin: "4px 0",
  },
};
