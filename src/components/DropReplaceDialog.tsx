import { DialogShell, dialogCancelStyle } from "./DialogShell";
import { useT } from "../i18n";

/**
 * Confirmation before a dropped file replaces the loaded model (issue #21):
 * a stray drop must not silently discard unexported painting — the same
 * stake the exit confirm guards (issue #7), so it rides the shared
 * DialogShell. Esc cancels; replacing is a deliberate click.
 */
export function DropReplaceDialog({
  onCancel,
  onConfirm,
}: {
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const t = useT();

  return (
    <DialogShell
      onCancel={onCancel}
      icon="⚠️"
      title={t("drop.replaceTitle")}
      body={t("drop.replaceBody")}
    >
      <button className="cym-btn" style={dialogCancelStyle} onClick={onCancel}>
        {t("drop.replaceCancel")}
      </button>
      <button className="cym-btn" style={styles.btnConfirm} onClick={onConfirm}>
        {t("drop.replaceConfirm")}
      </button>
    </DialogShell>
  );
}

const styles: Record<string, React.CSSProperties> = {
  btnConfirm: {
    padding: "6px 14px",
    border: "1px solid var(--accent-border, #2f6fdb)",
    background: "var(--accent, #3b82f6)",
    color: "#fff",
    borderRadius: 6,
    cursor: "pointer",
    fontSize: 13,
    fontWeight: 600,
  },
};
