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

  return (
    <div style={styles.container}>
      <div style={styles.header}>{t("brush.title")}</div>

      <div style={styles.row}>
        <label style={styles.label}>{t("brush.radius")}: {brushRadius.toFixed(1)}mm</label>
        <input
          type="range"
          min="0.5"
          max="50"
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
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  container: {
    padding: 10,
    background: "#2d2d2d",
    borderRadius: 8,
  },
  header: {
    color: "#ddd",
    fontSize: 14,
    fontWeight: 600,
    marginBottom: 8,
  },
  row: {
    marginBottom: 8,
  },
  label: {
    color: "#bbb",
    fontSize: 12,
    display: "block",
    marginBottom: 2,
  },
  slider: {
    width: "100%",
    cursor: "pointer",
  },
  select: {
    width: "100%",
    padding: "4px 6px",
    borderRadius: 4,
    border: "1px solid #555",
    background: "#3a3a3a",
    color: "#ddd",
    fontSize: 12,
  },
};
