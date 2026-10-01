import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ask } from "@tauri-apps/plugin-dialog";
import { Viewport } from "./components/Viewport/Viewport";
import { Toolbar } from "./components/Toolbar/Toolbar";
import { ColorPanel } from "./components/ColorPanel/ColorPanel";
import { SegmentsPanel } from "./components/SegmentsPanel/SegmentsPanel";
import { BrushSettings } from "./components/BrushSettings/BrushSettings";
import { StatusBar } from "./components/StatusBar/StatusBar";
import { DebugLogViewer } from "./components/DebugLogViewer";
import { useAppStore } from "./store/appStore";
import { useT } from "./i18n";

function App() {
  const theme = useAppStore((s) => s.theme);
  const language = useAppStore((s) => s.language);
  const t = useT();

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
        const { meshData, paintDirty } = useAppStore.getState();
        // Nothing at stake -> fall through to the default close path.
        if (!meshData || !paintDirty) return;
        event.preventDefault();
        const quit = await ask(t("exit.confirmBody"), {
          title: t("exit.confirmTitle"),
          kind: "warning",
        });
        if (quit) await getCurrentWindow().destroy();
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
    // t is re-created each render; language is the only real dependency.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [language]);

  return (
    <div style={styles.root}>
      {/* Main layout */}
      <div style={styles.main}>
        {/* Left toolbar */}
        <div style={styles.leftPanel}>
          <Toolbar />
        </div>

        {/* Center viewport */}
        <div style={styles.viewport}>
          <Viewport />
        </div>

        {/* Right panel */}
        <div style={styles.rightPanel}>
          <SegmentsPanel />
          <BrushSettings />
          <ColorPanel />
        </div>
      </div>

      {/* Bottom status bar */}
      <StatusBar />

      {/* In-app debug log viewer (iteration 59): release builds can't open
          devtools, so logs surface here instead. */}
      <DebugLogViewer />
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
