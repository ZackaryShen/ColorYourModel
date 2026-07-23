import { Viewport } from "./components/Viewport/Viewport";
import { Toolbar } from "./components/Toolbar/Toolbar";
import { ColorPanel } from "./components/ColorPanel/ColorPanel";
import { SegmentsPanel } from "./components/SegmentsPanel/SegmentsPanel";
import { BrushSettings } from "./components/BrushSettings/BrushSettings";
import { StatusBar } from "./components/StatusBar/StatusBar";

function App() {
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
    background: "#1e1e1e",
    color: "#eee",
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
