import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useAppStore } from "../store/appStore";
import { useT } from "../i18n";

/**
 * In-app replacement for the native exit MessageBox (issue #7): the OS dialog
 * is white/classic on Windows and clashes with the dark theme. Same visual
 * language as ExportDialog; Esc stays, only the red button quits.
 */
export function ExitConfirmDialog({ onCancel }: { onCancel: () => void }) {
  const t = useT();

  // Esc = stay. No Enter binding: quitting must be a deliberate click.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  return (
    <div style={styles.overlay} onClick={onCancel}>
      <div
        style={styles.dialog}
        onClick={(e) => e.stopPropagation()}
        role="alertdialog"
        aria-modal="true"
      >
        <div style={styles.titleRow}>
          <span style={styles.icon} aria-hidden>
            ⚠️
          </span>
          <div style={styles.header}>{t("exit.confirmTitle")}</div>
        </div>
        <div style={styles.body}>{t("exit.confirmBody")}</div>
        <div style={styles.btnRow}>
          <button className="cym-btn" style={styles.btnCancel} onClick={onCancel}>
            {t("exit.stay")}
          </button>
          <button
            className="cym-btn"
            style={styles.btnQuit}
            onClick={() => {
              getCurrentWindow().destroy();
            }}
          >
            {t("exit.quit")}
          </button>
        </div>
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  overlay: {
    position: "fixed",
    inset: 0,
    background: "rgba(0,0,0,0.55)",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    zIndex: 1100,
  },
  dialog: {
    width: 380,
    background: "var(--bg-panel, #2d2d2d)",
    border: "1px solid var(--border, #555)",
    borderRadius: 10,
    padding: 16,
    color: "var(--text-1, #eee)",
    fontFamily: "inherit",
    boxShadow: "0 8px 32px rgba(0,0,0,0.45)",
  },
  titleRow: {
    display: "flex",
    alignItems: "center",
    gap: 8,
    marginBottom: 10,
  },
  icon: {
    fontSize: 18,
    lineHeight: 1,
  },
  header: {
    fontSize: 15,
    fontWeight: 600,
  },
  body: {
    fontSize: 13,
    color: "var(--text-2, #aaa)",
    lineHeight: 1.6,
  },
  btnRow: {
    display: "flex",
    justifyContent: "flex-end",
    gap: 8,
    marginTop: 16,
  },
  btnCancel: {
    padding: "6px 14px",
    border: "1px solid var(--border, #555)",
    background: "var(--bg-elevated, #3a3a3a)",
    color: "var(--text-1, #eee)",
    borderRadius: 6,
    cursor: "pointer",
    fontSize: 13,
  },
  btnQuit: {
    padding: "6px 14px",
    border: "1px solid var(--danger-border, #b3373c)",
    background: "var(--danger, #e5484d)",
    color: "#fff",
    borderRadius: 6,
    cursor: "pointer",
    fontSize: 13,
    fontWeight: 600,
  },
};
