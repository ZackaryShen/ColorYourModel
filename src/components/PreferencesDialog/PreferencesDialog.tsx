import { useEffect } from "react";
import { useAppStore } from "../../store/appStore";
import { useT } from "../../i18n";
import { EXPERIMENTAL_FEATURES } from "../../experimentalFeatures";

/**
 * Preferences dialog (developer mode): one checkbox per entry of
 * EXPERIMENTAL_FEATURES, persisted through the store's cym-prefs record.
 * Same visual language as ExitConfirmDialog (dark modal, Esc / × / backdrop
 * click to close). No danger actions here, so Esc and the close button are
 * equivalent — toggles apply immediately on change.
 */
export function PreferencesDialog({ onClose }: { onClose: () => void }) {
  const t = useT();
  const experimentalFeatures = useAppStore((s) => s.experimentalFeatures);
  const setExperimentalFeature = useAppStore((s) => s.setExperimentalFeature);

  // Esc closes. Toggles commit on change, so there is no confirm/cancel pair.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div style={styles.overlay} onClick={onClose}>
      <div
        style={styles.dialog}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label={t("prefs.title")}
      >
        <div style={styles.titleRow}>
          <span style={styles.icon} aria-hidden>
            ⚙️
          </span>
          <div style={styles.header}>{t("prefs.title")}</div>
          <button
            className="cym-btn"
            style={styles.closeBtn}
            onClick={onClose}
            title={t("prefs.close")}
            aria-label={t("prefs.close")}
          >
            ×
          </button>
        </div>

        <div style={styles.sectionTitle}>{t("prefs.experimentalSection")}</div>
        <div style={styles.sectionHint}>{t("prefs.experimentalHint")}</div>

        <div style={styles.featureList}>
          {EXPERIMENTAL_FEATURES.map((f) => (
            <label key={f.id} style={styles.featureRow}>
              <input
                type="checkbox"
                checked={experimentalFeatures[f.id]}
                onChange={(e) => setExperimentalFeature(f.id, e.target.checked)}
                style={styles.checkbox}
              />
              <span style={styles.featureText}>
                <span style={styles.featureLabel}>{t(f.labelKey)}</span>
                <span style={styles.featureHint}>{t(f.hintKey)}</span>
              </span>
            </label>
          ))}
        </div>

        <div style={styles.btnRow}>
          <button className="cym-btn" style={styles.btnDone} onClick={onClose}>
            {t("prefs.done")}
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
    width: 440,
    maxHeight: "80vh",
    overflowY: "auto",
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
    marginBottom: 12,
  },
  icon: { fontSize: 18 },
  header: {
    fontSize: 15,
    fontWeight: 600,
    flex: 1,
  },
  closeBtn: {
    width: 24,
    height: 24,
    padding: 0,
    lineHeight: "20px",
    fontSize: 14,
    borderRadius: 6,
    border: "1px solid var(--border, #555)",
    background: "transparent",
    color: "var(--text-2, #aaa)",
    cursor: "pointer",
  },
  sectionTitle: {
    fontSize: 13,
    fontWeight: 600,
    color: "var(--text-1, #ddd)",
    margin: "8px 0 4px",
  },
  sectionHint: {
    fontSize: 12,
    lineHeight: 1.5,
    color: "var(--text-3, #888)",
    marginBottom: 10,
  },
  featureList: {
    display: "flex",
    flexDirection: "column",
    gap: 6,
  },
  featureRow: {
    display: "flex",
    alignItems: "flex-start",
    gap: 8,
    padding: "6px 8px",
    borderRadius: 6,
    border: "1px solid var(--border, #444)",
    background: "var(--bg-elevated, #333)",
    cursor: "pointer",
  },
  checkbox: {
    marginTop: 3,
    accentColor: "var(--accent, #4a9eff)",
    cursor: "pointer",
  },
  featureText: {
    display: "flex",
    flexDirection: "column",
    gap: 2,
  },
  featureLabel: {
    fontSize: 13,
    fontWeight: 600,
    color: "var(--text-1, #ddd)",
  },
  featureHint: {
    fontSize: 11,
    lineHeight: 1.5,
    color: "var(--text-3, #888)",
  },
  btnRow: {
    display: "flex",
    justifyContent: "flex-end",
    marginTop: 14,
  },
  btnDone: {
    padding: "6px 16px",
    borderRadius: 6,
    border: "1px solid var(--border, #555)",
    background: "var(--bg-elevated, #3a3a3a)",
    color: "var(--text-1, #ccc)",
    cursor: "pointer",
  },
};
