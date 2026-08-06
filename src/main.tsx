import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./theme.css";

// Measurement escape hatch for the Phase 0 gates (Gate 0a, instrument
// calibration). StrictMode double-invokes every useMemo factory in
// development, so on each mesh load the 54 MB colour buffers and the
// non-indexed geometry expansion are built TWICE. The inflation is
// deterministic, which is exactly what makes it dangerous: repeated runs agree
// with each other, so the numbers look trustworthy while sitting at roughly
// twice the cost the shipped build actually pays.
//
// Run `VITE_NO_STRICT=1 cargo tauri dev` when taking a measurement; every other
// run keeps StrictMode on so genuine purity violations still surface.
const strictMode = import.meta.env.VITE_NO_STRICT !== "1";

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);

root.render(
  strictMode ? (
    <React.StrictMode>
      <App />
    </React.StrictMode>
  ) : (
    <App />
  )
);
