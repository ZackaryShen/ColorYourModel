import { useT } from "../i18n";

/**
 * Full-window "release to import" highlight shown while files are dragged
 * over the app (issue #21). The drop zone is the WHOLE window, not just the
 * viewport, so the overlay covers everything; `pointerEvents: none` keeps it
 * from intercepting anything (the Tauri drag events it visualizes are
 * webview-level, not DOM-level).
 */
export function DropOverlay() {
  const t = useT();
  return (
    <div style={styles.overlay}>
      <div style={styles.badge}>{t("drop.hint")}</div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  overlay: {
    position: "fixed",
    inset: 0,
    zIndex: 1000,
    pointerEvents: "none",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    background: "rgba(0, 0, 0, 0.35)",
    border: "3px dashed var(--accent, #4a9eff)",
    borderRadius: 6,
  },
  badge: {
    padding: "10px 22px",
    borderRadius: 8,
    background: "var(--bg-panel, #2d2d2d)",
    border: "1px solid var(--border, #555)",
    color: "var(--text-1, #eee)",
    fontSize: 15,
    fontWeight: 600,
    boxShadow: "0 8px 32px rgba(0,0,0,0.45)",
  },
};
