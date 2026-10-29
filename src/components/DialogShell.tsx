import { useEffect } from "react";
import type { ReactNode } from "react";

/**
 * Shared modal shell for the in-app confirmations (exit #7, drop-replace
 * #21): dimmed overlay + bordered dark panel + Esc-to-cancel. Owns the
 * structure and styles the exit confirm established; callers supply title,
 * body and their own action buttons (the primary action's styling is
 * deliberately theirs — quitting is red, replacing is accent blue).
 */
export function DialogShell({
  onCancel,
  icon,
  title,
  body,
  children,
}: {
  onCancel: () => void;
  icon: string;
  title: string;
  body: string;
  children: ReactNode;
}) {
  // Esc = cancel. The primary action must be a deliberate click.
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
            {icon}
          </span>
          <div style={styles.header}>{title}</div>
        </div>
        <div style={styles.body}>{body}</div>
        <div style={styles.btnRow}>{children}</div>
      </div>
    </div>
  );
}

/** Shared secondary-button style; primary buttons stay with their dialogs. */
export const dialogCancelStyle: React.CSSProperties = {
  padding: "6px 14px",
  border: "1px solid var(--border, #555)",
  background: "var(--bg-elevated, #3a3a3a)",
  color: "var(--text-1, #eee)",
  borderRadius: 6,
  cursor: "pointer",
  fontSize: 13,
};

/**
 * Shared danger-primary style (exit quit, reset wipe #28): the destructive
 * action reads as red everywhere in the app.
 */
export const dialogDangerStyle: React.CSSProperties = {
  padding: "6px 14px",
  border: "1px solid var(--danger-border, #b3373c)",
  background: "var(--danger, #e5484d)",
  color: "#fff",
  borderRadius: 6,
  cursor: "pointer",
  fontSize: 13,
  fontWeight: 600,
};

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
};
