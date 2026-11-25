import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Viewport } from "./components/Viewport/Viewport";
import { Toolbar } from "./components/Toolbar/Toolbar";
import { ColorPanel } from "./components/ColorPanel/ColorPanel";
import { SegmentsPanel } from "./components/SegmentsPanel/SegmentsPanel";
import { BrushSettings } from "./components/BrushSettings/BrushSettings";
import { StatusBar } from "./components/StatusBar/StatusBar";
import { MenuBar } from "./components/MenuBar";
import { DebugLogViewer } from "./components/DebugLogViewer";
import { ExitConfirmDialog } from "./components/ExitConfirmDialog";
import { ImportErrorDialog } from "./components/ImportErrorDialog";
import { PreferencesDialog } from "./components/PreferencesDialog/PreferencesDialog";
import { HistoryPanel } from "./components/HistoryPanel";
import { DropOverlay } from "./components/DropOverlay";
import { DropReplaceDialog } from "./components/DropReplaceDialog";
import { UpdateDialog } from "./components/UpdateDialog/UpdateDialog";
import { useAutoUpdateCheck } from "./hooks/useAutoUpdateCheck";
import { useDropImport } from "./hooks/useDropImport";
import { useImportModel } from "./hooks/useImportModel";
import { useTextureImport } from "./hooks/useTextureImport";
import { TexturePreviewDialog } from "./components/TexturePreview/TexturePreviewDialog";
import { useAppStore } from "./store/appStore";

function App() {
  const theme = useAppStore((s) => s.theme);
  const language = useAppStore((s) => s.language);
  // GUI audit 2026-10-02 (B14): the brush controls do nothing without a mesh;
  // showing them on the empty state invited dead-slider fiddling.
  const hasMesh = useAppStore((s) => s.meshData !== null);
  const historyPanelOpen = useAppStore((s) => s.historyPanelOpen);
  const textureImport = useTextureImport();
  const { pickModel, importFromPath, isImportBusy } = useImportModel(textureImport);
  const { dragOver, pendingReplacePath, confirmReplace, cancelReplace } =
    useDropImport(importFromPath, isImportBusy);
  const [confirmExit, setConfirmExit] = useState(false);
  const [prefsOpen, setPrefsOpen] = useState(false);

  // Silent startup update check (dev builds and opted-out users never fire).
  useAutoUpdateCheck();

  // Sync the chosen theme to <html data-theme> so the static CSS variables in
  // theme.css resolve. No FOUC: theme.css is imported in main.tsx at load.
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  // Issue #7: with a close-requested listener attached, Tauri never
  // auto-closes the window - we must destroy() it ourselves. Re-subscribed on
  // language change so the dialog text always matches the active locale.
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    getCurrentWindow()
      .onCloseRequested(async (event) => {
        // Applying changes the active backend mesh. Keep the window open
        // until the renderer receives the same committed mesh.
        if (textureImport.isApplying()) {
          event.preventDefault();
          return;
        }
        if (textureImport.isOpen()) textureImport.cancel();
        const { meshData, paintDirty } = useAppStore.getState();
        // Nothing at stake -> fall through to the default close path.
        if (!meshData || !paintDirty) return;
        event.preventDefault();
        setConfirmExit(true);
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
    // Texture callbacks only read stable refs and update through a stable setter.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [language]);

  return (
    <div style={styles.root}>
      {/* Top menu bar (File / View / Help) — conventional desktop entry
          points alongside the left tool Toolbar. */}
      <MenuBar onOpenPreferences={() => setPrefsOpen(true)} onImport={pickModel} />

      {/* Main layout */}
      <div style={styles.main}>
        {/* Left toolbar */}
        <div style={styles.leftPanel}>
          <Toolbar onImport={pickModel} />
        </div>

        {/* Center viewport */}
        <div style={styles.viewport}>
          <Viewport />
        </div>

        {/* Issue #22: Photoshop-style history panel, docked as an extra
            column; toggled from 视图 → 历史记录. */}
        {historyPanelOpen && <HistoryPanel />}

        {/* Right panel */}
        <div style={styles.rightPanel}>
          <SegmentsPanel />
          {hasMesh && <BrushSettings />}
          <ColorPanel />
        </div>
      </div>

      {/* Bottom status bar */}
      <StatusBar onOpenPreferences={() => setPrefsOpen(true)} />

      {/* In-app debug log viewer (iteration 59): release builds can't open
          devtools, so logs surface here instead. */}
      <DebugLogViewer />

      {/* Issue #7: in-app exit confirmation (dark-themed, replaces the
          white native MessageBox). */}
      {confirmExit && <ExitConfirmDialog onCancel={() => setConfirmExit(false)} />}

      {/* Developer mode: experimental-feature toggles (per-feature checkboxes,
          persisted via cym-prefs). */}
      {prefsOpen && <PreferencesDialog onClose={() => setPrefsOpen(false)} />}

      {/* P0 2026-10-12: import failures (broken/unsupported STL or .cym)
          surface as a modal instead of only a status-bar line. */}
      <ImportErrorDialog />

      {/* Issue #21: whole-window drop highlight while files hover, and the
          replace confirmation for a drop onto a loaded model. */}
      {dragOver && <DropOverlay />}
      {pendingReplacePath && (
        <DropReplaceDialog onCancel={cancelReplace} onConfirm={confirmReplace} />
      )}

      {textureImport.session && (
        <TexturePreviewDialog
          session={textureImport.session}
          hasModel={hasMesh}
          onApply={textureImport.apply}
          onCancel={textureImport.cancel}
          onColorLimit={textureImport.setColorLimit}
          onRetry={textureImport.retry}
        />
      )}

      {/* Update module: renders nothing while phase === "idle"; owns its own
          phases (available / downloading / ready / installing / error). */}
      <UpdateDialog />
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  root: {
    display: "flex",
    flexDirection: "column",
    height: "100vh",
    width: "100vw",
    overflow: "hidden",
    background: "var(--bg-root, #1e1e1e)",
    color: "var(--text-1, #eeeeee)",
    fontFamily:
      '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
  },
  main: {
    display: "flex",
    flex: 1,
    overflow: "hidden",
  },
  leftPanel: {
    display: "flex",
    padding: 8,
    overflowY: "auto",
  },
  viewport: {
    flex: 1,
    position: "relative",
    overflow: "hidden",
  },
  rightPanel: {
    width: 220,
    padding: 8,
    display: "flex",
    flexDirection: "column",
    gap: 8,
    overflowY: "auto",
  },
};

export default App;
