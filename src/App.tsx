import { useEffect } from "react";
import { Viewport } from "./components/Viewport/Viewport";
import { Toolbar } from "./components/Toolbar/Toolbar";
import { ColorPanel } from "./components/ColorPanel/ColorPanel";
import { SegmentsPanel } from "./components/SegmentsPanel/SegmentsPanel";
import { BrushSettings } from "./components/BrushSettings/BrushSettings";
import { StatusBar } from "./components/StatusBar/StatusBar";
import { useAppStore } from "./store/appStore";

function App() {
  const theme = useAppStore((s) => s.theme);

  // Sync the chosen theme to <html data-theme> so the static CSS variables in
  // theme.css resolve. No FOUC: theme.css is imported in main.tsx at load.
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

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
