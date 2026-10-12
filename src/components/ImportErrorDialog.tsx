import { useEffect } from "react";
import { useAppStore } from "../store/appStore";
import { useT } from "../i18n";

/**
 * Modal shown when an STL/.cym import fails (P0 2026-10-12): a broken or
 * unsupported file used to die as a status-bar line at best and a Rust panic
 * at worst. Same visual language as ExitConfirmDialog. The raw backend error
 * travels in store.importError; Esc / overlay click / OK all dismiss it.
 */
export function ImportErrorDialog() {
  const t = useT();
  const message = useAppStore((s) => s.importError);
  const setImportError = useAppStore((s) => s.setImportError);
  const close = () => setImportError(null);

  // Hooks BEFORE any conditional return (cf. c1ae169: conditional hook calls
  // are React error #321). The listener is harmless while the dialog is closed.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (!message) return null;

  return (
    <div style={styles.overlay} onClick={close}>
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
          <div style={styles.header}>{t("dialog.importFailedTitle")}</div>
        </div>
        <div style={styles.body}>{message}</div>
        <div style={styles.btnRow}>
          <button className="cym-btn" style={styles.btnOk} onClick={close}>
            {t("dialog.ok")}
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
    width: 420,
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
    // Backend error strings carry file paths / parser details — let them wrap
    // instead of stretching the dialog.
    overflowWrap: "anywhere",
    maxHeight: "40vh",
    overflowY: "auto",
    whiteSpace: "pre-wrap",
  },
  btnRow: {
    display: "flex",
    justifyContent: "flex-end",
    gap: 8,
    marginTop: 16,
  },
  btnOk: {
    padding: "6px 14px",
    border: "1px solid var(--border, #555)",
    background: "var(--bg-elevated, #3a3a3a)",
    color: "var(--text-1, #eee)",
    borderRadius: 6,
    cursor: "pointer",
    fontSize: 13,
  },
};
