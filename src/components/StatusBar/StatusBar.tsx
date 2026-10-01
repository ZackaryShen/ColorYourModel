import { useAppStore } from "../../store/appStore";
import { useT } from "../../i18n";

export function StatusBar() {
  const t = useT();
  const statusMessage = useAppStore((s) => s.statusMessage);
  const meshData = useAppStore((s) => s.meshData);
  const activeTool = useAppStore((s) => s.activeTool);
  const language = useAppStore((s) => s.language);
  const setLanguage = useAppStore((s) => s.setLanguage);
  const theme = useAppStore((s) => s.theme);
  const setTheme = useAppStore((s) => s.setTheme);
  const debugLogOpen = useAppStore((s) => s.debugLogOpen);
  const setDebugLogOpen = useAppStore((s) => s.setDebugLogOpen);

  return (
    <div style={styles.container}>
      {/* M8: statusMessage may be an i18n key (the boot default) or a runtime
          literal pushed by setStatusMessage. `t()` returns unknown keys
          verbatim, so both paths render correctly. */}
      <span style={styles.status}>{t(statusMessage)}</span>
      <span style={styles.info}>
        {t("status.tool")}: <strong>{activeTool}</strong>
      </span>
      {meshData && (
        <span style={styles.info}>
          {t("status.faces")}: <strong>{meshData.faceCount.toLocaleString()}</strong>
        </span>
      )}
      <span style={styles.tip}>{t("status.tip")}</span>
      {/* Debug log toggle joins the theme/language row: the old floating
          bottom-right 🐞 button sat on top of the language switch. */}
      <button
        onClick={() => setDebugLogOpen(!debugLogOpen)}
        className="cym-btn"
        style={{ ...styles.themeBtn, ...(debugLogOpen ? styles.debugActive : {}) }}
        title={t("debugLog.openTitle")}
      >
        🐞
      </button>
      <button
        onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
        className="cym-btn"
        style={styles.themeBtn}
        title={t("theme.toggle")}
      >
        {theme === "dark" ? "☀️" : "🌙"}
      </button>
      <button
        onClick={() => setLanguage(language === "zh" ? "en" : "zh")}
        className="cym-btn"
        style={styles.langBtn}
        title={language === "zh" ? "Switch to English" : "切换到中文"}
      >
        {t("lang.switch")}
      </button>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  container: {
    display: "flex",
    alignItems: "center",
    gap: 16,
    padding: "4px 12px",
    background: "var(--bg-elevated, #252525)",
    color: "var(--text-2, #aaaaaa)",
    fontSize: 12,
    borderTop: "1px solid var(--border-strong, #333333)",
  },
  status: {
    flex: 1,
  },
  info: {
    whiteSpace: "nowrap" as const,
  },
  tip: {
    color: "var(--text-3, #666666)",
    whiteSpace: "nowrap" as const,
  },
  themeBtn: {
    padding: "2px 8px",
    border: "1px solid var(--border, #555555)",
    borderRadius: 4,
    background: "var(--bg-elevated, #3a3a3a)",
    color: "var(--text-1, #cccccc)",
    cursor: "pointer",
    fontSize: 12,
    whiteSpace: "nowrap" as const,
  },
  debugActive: {
    borderColor: "var(--accent, #4a9eff)",
    color: "var(--accent, #4a9eff)",
  },
  langBtn: {
    padding: "2px 8px",
    border: "1px solid var(--border, #555555)",
    borderRadius: 4,
    background: "var(--bg-elevated, #3a3a3a)",
    color: "var(--text-1, #cccccc)",
    cursor: "pointer",
    fontSize: 11,
    fontWeight: 600,
    whiteSpace: "nowrap" as const,
  },
};
