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

// Diagnostic: confirm React mounted. Surfaces in headless/release builds via
// the JS→Rust bridge (window.__TAURI_INTERNALS__) so a blank window is no
// longer a silent mystery. Harmless in plain browsers (guard below).
try {
  const ti = (window as unknown as {
    __TAURI_INTERNALS__?: { invoke?: (c: string, a?: unknown) => Promise<unknown> };
  }).__TAURI_INTERNALS__;
  const invoke = ti?.invoke;
  if (typeof invoke === "function") {
    setTimeout(() => {
      try {
        invoke("report_app_ready");
      } catch {
        /* ignore */
      }
    }, 1500);
  }
} catch {
  /* ignore */
}
