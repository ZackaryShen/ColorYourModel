import { useAppStore } from "../../store/appStore";
import { useT } from "../../i18n";

export function BrushSettings() {
  const t = useT();
  const brushRadius = useAppStore((s) => s.brushRadius);
  const brushStrength = useAppStore((s) => s.brushStrength);
  const brushFalloff = useAppStore((s) => s.brushFalloff);
  const setBrushRadius = useAppStore((s) => s.setBrushRadius);
  const setBrushStrength = useAppStore((s) => s.setBrushStrength);
  const setBrushFalloff = useAppStore((s) => s.setBrushFalloff);
  const shadingMode = useAppStore((s) => s.shadingMode);
  const setShadingMode = useAppStore((s) => s.setShadingMode);

  return (
    <div style={styles.container}>
      <div style={styles.header}>{t("brush.title")}</div>

      <div style={styles.row}>
        <label style={styles.label}>{t("brush.radius")}: {brushRadius.toFixed(1)}mm</label>
        <input
          type="range"
          min="0.5"
          max="200"
          step="0.5"
          value={brushRadius}
          onChange={(e) => setBrushRadius(parseFloat(e.target.value))}
          style={styles.slider}
        />
      </div>

      <div style={styles.row}>
        <label style={styles.label}>
          {t("brush.strength")}: {(brushStrength * 100).toFixed(0)}%
        </label>
        <input
          type="range"
          min="0.1"
          max="1.0"
          step="0.05"
          value={brushStrength}
          onChange={(e) => setBrushStrength(parseFloat(e.target.value))}
          style={styles.slider}
        />
      </div>

      <div style={styles.row}>
        <label style={styles.label}>{t("brush.falloff")}:</label>
        <select
          value={brushFalloff}
          onChange={(e) =>
            setBrushFalloff(e.target.value as "linear" | "smooth" | "step")
          }
          style={styles.select}
        >
          <option value="smooth">{t("brush.smooth")}</option>
          <option value="linear">{t("brush.linear")}</option>
          <option value="step">{t("brush.step")}</option>
        </select>
      </div>

      <div style={styles.row}>
        <label style={styles.label}>{t("shading.mode")}</label>
        <div style={styles.toggleGroup}>
          <button
            className="cym-toggle"
            style={{
              ...styles.toggleBtn,
              ...(shadingMode === "flat" ? styles.toggleActive : {}),
            }}
            onClick={() => setShadingMode("flat")}
          >
            {t("shading.flat")}
          </button>
          <button
            className="cym-toggle"
            style={{
              ...styles.toggleBtn,
              ...(shadingMode === "shaded" ? styles.toggleActive : {}),
            }}
            onClick={() => setShadingMode("shaded")}
          >
            {t("shading.shaded")}
          </button>
        </div>
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  container: {
    padding: 10,
    background: "var(--bg-panel, #2d2d2d)",
    borderRadius: 8,
  },
  header: {
    color: "var(--text-1, #dddddd)",
    fontSize: 14,
    fontWeight: 600,
    marginBottom: 8,
  },
  row: {
    marginBottom: 8,
  },
  label: {
    color: "var(--text-2, #bbbbbb)",
    fontSize: 12,
    display: "block",
    marginBottom: 2,
  },
  slider: {
    width: "100%",
    cursor: "pointer",
    accentColor: "var(--accent, #4a9eff)",
  },
  select: {
    width: "100%",
    padding: "4px 6px",
    borderRadius: 4,
    border: "1px solid var(--border, #555555)",
    background: "var(--bg-elevated, #3a3a3a)",
    color: "var(--text-1, #dddddd)",
    fontSize: 12,
  },
  toggleGroup: {
    display: "flex",
    gap: 4,
  },
  toggleBtn: {
    flex: 1,
    padding: "5px 8px",
    borderRadius: 4,
    border: "1px solid var(--border, #555555)",
    background: "var(--bg-panel, #2d2d2d)",
    color: "var(--text-2, #aaaaaa)",
    fontSize: 11,
    cursor: "pointer",
    textAlign: "center" as const,
  },
  toggleActive: {
    background: "var(--accent-strong, #4a6fa5)",
    color: "var(--text-1, #ffffff)",
    borderColor: "var(--accent-border, #6a9fd5)",
  },
};
