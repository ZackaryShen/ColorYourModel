import { getCurrentWindow } from "@tauri-apps/api/window";
import { DialogShell, dialogCancelStyle } from "./DialogShell";
import { useT } from "../i18n";

/**
 * In-app replacement for the native exit MessageBox (issue #7): the OS dialog
 * is white/classic on Windows and clashes with the dark theme. Same visual
 * language as ExportDialog; Esc stays, only the red button quits.
 */
export function ExitConfirmDialog({ onCancel }: { onCancel: () => void }) {
  const t = useT();

  return (
    <DialogShell
      onCancel={onCancel}
      icon="⚠️"
      title={t("exit.confirmTitle")}
      body={t("exit.confirmBody")}
    >
      <button className="cym-btn" style={dialogCancelStyle} onClick={onCancel}>
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
    </DialogShell>
  );
}

const styles: Record<string, React.CSSProperties> = {
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
