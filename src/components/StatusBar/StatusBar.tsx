import { useAppStore } from "../../store/appStore";
import { useT } from "../../i18n";

export function StatusBar() {
  const t = useT();
  const statusMessage = useAppStore((s) => s.statusMessage);
  const meshData = useAppStore((s) => s.meshData);
  const activeTool = useAppStore((s) => s.activeTool);
  const language = useAppStore((s) => s.language);
  const setLanguage = useAppStore((s) => s.setLanguage);

  return (
    <div style={styles.container}>
      <span style={styles.status}>{statusMessage}</span>
      <span style={styles.info}>
        {t("status.tool")}: <strong>{activeTool}</strong>
      </span>
      {meshData && (
        <span style={styles.info}>
          {t("status.faces")}: <strong>{meshData.faceCount.toLocaleString()}</strong>
        </span>
      )}
      <span style={styles.tip}>{t("status.tip")}</span>
      <button
        onClick={() => setLanguage(language === "zh" ? "en" : "zh")}
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
    background: "#252525",
    color: "#aaa",
    fontSize: 12,
    borderTop: "1px solid #333",
  },
  status: {
    flex: 1,
  },
  info: {
    whiteSpace: "nowrap" as const,
  },
  tip: {
    color: "#666",
    whiteSpace: "nowrap" as const,
  },
  langBtn: {
    padding: "2px 8px",
    border: "1px solid #555",
    borderRadius: 4,
    background: "#3a3a3a",
    color: "#ccc",
    cursor: "pointer",
    fontSize: 11,
    fontWeight: 600,
    whiteSpace: "nowrap" as const,
  },
};
