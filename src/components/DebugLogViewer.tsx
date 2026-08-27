import { useEffect, useRef, useState } from "react";
import { subscribeLog, type LogEntry } from "../utils/logger";

/// In-app debug log viewer (iteration 59). Tauri release builds disable the
/// webview devtools, so the existing console.* diagnostics are invisible to the
/// user. This panel renders the same log ring directly in the app so the
/// seed-click / segmentation diagnostics can be read without devtools.
///
/// Toggle with the keyboard shortcut Ctrl+Shift+L (or the floating "🐞" button
/// at the bottom-right). Auto-scrolls to the newest entry.
export function DebugLogViewer() {
  const [open, setOpen] = useState(false);
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    // Hotkey: Ctrl+Shift+L toggles the panel.
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.shiftKey && (e.key === "L" || e.key === "l")) {
        e.preventDefault();
        setOpen((o) => !o);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    if (!open) return;
    const unsub = subscribeLog(setEntries);
    return unsub;
  }, [open]);

  useEffect(() => {
    if (open && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [entries, open]);

  return (
    <>
      {/* Floating toggle button */}
      <button
        onClick={() => setOpen((o) => !o)}
        title="调试日志 (Ctrl+Shift+L)"
        style={{
          position: "fixed",
          bottom: 12,
          right: 12,
          zIndex: 9999,
          width: 36,
          height: 36,
          borderRadius: "50%",
          border: "1px solid var(--border, #555)",
          background: "var(--bg-panel, #2d2d2d)",
          color: open ? "#4a9eff" : "var(--text-2, #ccc)",
          cursor: "pointer",
          fontSize: 16,
          lineHeight: 1,
        }}
      >
        🐞
      </button>

      {open && (
        <div
          style={{
            position: "fixed",
            bottom: 56,
            right: 12,
            zIndex: 9999,
            width: 460,
            maxWidth: "92vw",
            height: 320,
            maxHeight: "70vh",
            display: "flex",
            flexDirection: "column",
            background: "rgba(20,20,20,0.96)",
            border: "1px solid var(--accent, #4a9eff)",
            borderRadius: 8,
            boxShadow: "0 8px 32px rgba(0,0,0,0.6)",
            overflow: "hidden",
          }}
        >
          <div
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              padding: "6px 10px",
              borderBottom: "1px solid var(--border, #333)",
              color: "#ccc",
              fontSize: 12,
            }}
          >
            <span>🐞 调试日志（{entries.length} 条）</span>
            <button
              onClick={() => setEntries([])}
              style={{
                background: "transparent",
                border: "1px solid var(--border, #555)",
                color: "#aaa",
                borderRadius: 4,
                padding: "2px 8px",
                cursor: "pointer",
                fontSize: 11,
              }}
            >
              清空
            </button>
          </div>
          <div
            ref={scrollRef}
            style={{
              flex: 1,
              overflowY: "auto",
              padding: 8,
              fontFamily: "ui-monospace, Menlo, Consolas, monospace",
              fontSize: 11,
              lineHeight: 1.45,
            }}
          >
            {entries.length === 0 && (
              <div style={{ color: "#777" }}>暂无日志</div>
            )}
            {entries.map((e, i) => (
              <div
                key={i}
                style={{
                  color:
                    e.level === "error"
                      ? "#ff6b6b"
                      : e.level === "warn"
                      ? "#ffb84d"
                      : e.level === "info"
                      ? "#7ec8ff"
                      : "#bbb",
                  whiteSpace: "pre-wrap",
                  wordBreak: "break-all",
                }}
              >
                <span style={{ opacity: 0.6 }}>{e.ts}</span>{" "}
                <span style={{ fontWeight: 700 }}>[{e.tag}]</span> {e.msg}
                {e.data !== undefined && (
                  <div style={{ opacity: 0.7, paddingLeft: 8 }}>
                    {JSON.stringify(e.data)}
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>
      )}
    </>
  );
}
