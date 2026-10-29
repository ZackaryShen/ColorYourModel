import { getCurrentWindow } from "@tauri-apps/api/window";
import { DialogShell, dialogCancelStyle, dialogDangerStyle } from "./DialogShell";
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
        style={dialogDangerStyle}
        onClick={() => {
          getCurrentWindow().destroy();
        }}
      >
        {t("exit.quit")}
      </button>
    </DialogShell>
  );
}
