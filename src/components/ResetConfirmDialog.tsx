import { createPortal } from "react-dom";
import { DialogShell, dialogCancelStyle, dialogDangerStyle } from "./DialogShell";
import { useT } from "../i18n";

/**
 * In-app confirmation for the destructive partition reset (issue #28).
 * Both 清空分区 (Seed panel) and 重置分区 (Segments panel) used to guard
 * themselves with `window.confirm` — but in the Tauri WebView that call is
 * intercepted by tauri-plugin-dialog: it resolves asynchronously and
 * `plugin:dialog|confirm` is not even in the capability ACL, so the sync
 * guard saw a truthy Promise and the wipe ran unasked under an error
 * banner. This dialog has no ACL dependency, matches the app theme and the
 * other DialogShell confirmations, and the reset only ever fires from a
 * deliberate click on the danger button.
 *
 * Portalled to document.body so the overlay sits at the ROOT stacking
 * level even when a host panel (SeedPanel is zIndex 30, position:fixed)
 * would otherwise trap it in its own stacking context.
 */
export function ResetConfirmDialog({
  running,
  onCancel,
  onConfirm,
}: {
  running: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const t = useT();

  return createPortal(
    <DialogShell
      onCancel={running ? () => {} : onCancel}
      icon="⚠️"
      title={t("reset.confirmTitle")}
      body={t("segments.resetConfirm")}
    >
      <button
        className="cym-btn"
        style={dialogCancelStyle}
        onClick={onCancel}
        disabled={running}
      >
        {t("reset.cancel")}
      </button>
      <button className="cym-btn" style={dialogDangerStyle} onClick={onConfirm} disabled={running}>
        {running ? t("reset.resetting") : t("reset.confirm")}
      </button>
    </DialogShell>,
    document.body
  );
}
