import { useAppStore } from "../../store/appStore";
import { AMS_PALETTE } from "../../types/mesh";
import { useT } from "../../i18n";

export function ColorPanel() {
  const t = useT();
  const currentColor = useAppStore((s) => s.currentColor);
  const setCurrentColor = useAppStore((s) => s.setCurrentColor);

  const handleColorChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const hex = e.target.value;
    const r = parseInt(hex.slice(1, 3), 16);
    const g = parseInt(hex.slice(3, 5), 16);
    const b = parseInt(hex.slice(5, 7), 16);
    setCurrentColor([r, g, b, 255]);
  };

  const hexColor = `#${currentColor[0].toString(16).padStart(2, "0")}${currentColor[1].toString(16).padStart(2, "0")}${currentColor[2].toString(16).padStart(2, "0")}`;

  // Compute perceived brightness for UI contrast (ITU-R BT.709 luma)
  const luma = (0.2126 * currentColor[0] + 0.7152 * currentColor[1] + 0.0722 * currentColor[2]) / 255;
  const isDark = luma < 0.25;

  return (
    <div style={styles.container}>
      <div style={styles.header}>{t("color.title")}</div>
      <div style={styles.currentRow}>
        <div
          style={{
            ...styles.preview,
            backgroundColor: hexColor,
            borderColor: isDark ? "var(--accent, #4a9eff)" : "var(--border, #555555)",
            borderWidth: isDark ? 2 : 1,
            boxShadow: isDark ? "0 0 0 1px var(--bg-panel, #2d2d2d) inset" : "none",
          }}
          title={`RGB(${currentColor[0]}, ${currentColor[1]}, ${currentColor[2]})`}
        />
        <span style={{
          fontSize: 11,
          fontFamily: "monospace",
          color: "var(--text-2, #aaaaaa)",
          minWidth: 62,
        }}>
          {hexColor.toUpperCase()}
        </span>
        <input
          type="color"
          value={hexColor}
          onChange={handleColorChange}
          style={styles.colorInput}
        />
      </div>

      <div style={styles.subheader}>{t("color.amsPalette")}</div>
      <div style={styles.palette}>
        {AMS_PALETTE.map((entry) => {
          const hex = `#${entry.color[0].toString(16).padStart(2, "0")}${entry.color[1].toString(16).padStart(2, "0")}${entry.color[2].toString(16).padStart(2, "0")}`;
          const isActive =
            currentColor[0] === entry.color[0] &&
            currentColor[1] === entry.color[1] &&
            currentColor[2] === entry.color[2];
          return (
            <button
              key={entry.name}
              title={entry.name}
              onClick={() =>
                setCurrentColor([entry.color[0], entry.color[1], entry.color[2], 255])
              }
              className="cym-btn"
              style={{
                ...styles.swatch,
                backgroundColor: hex,
                border: isActive ? "2px solid var(--accent, #4a9eff)" : "1px solid var(--border, #555555)",
              }}
            />
          );
        })}
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
  subheader: {
    color: "var(--text-2, #aaaaaa)",
    fontSize: 12,
    marginTop: 8,
    marginBottom: 4,
  },
  currentRow: {
    display: "flex",
    alignItems: "center",
    gap: 8,
  },
  preview: {
    width: 32,
    height: 32,
    borderRadius: 6,
    border: "1px solid var(--border, #555555)",
  },
  colorInput: {
    width: 40,
    height: 32,
    border: "none",
    padding: 0,
    cursor: "pointer",
    background: "transparent",
  },
  palette: {
    display: "grid",
    gridTemplateColumns: "repeat(5, 1fr)",
    gap: 4,
  },
  swatch: {
    width: 28,
    height: 28,
    borderRadius: 4,
    cursor: "pointer",
  },
};
