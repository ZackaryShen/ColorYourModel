import { useAppStore } from "../../store/appStore";
import { useT } from "../../i18n";

const SEGMENT_COLORS = [
  "#e74c3c", "#3498db", "#2ecc71", "#f1c40f", "#9b59b6",
  "#e67e22", "#1abc9c", "#e91e63", "#00bcd4", "#8bc34a",
  "#ff9800", "#795548", "#607d8b", "#ff5722", "#673ab7",
];

export function SegmentsPanel() {
  const t = useT();
  const segments = useAppStore((s) => s.segments);
  const selectedSegment = useAppStore((s) => s.selectedSegment);
  const setSelectedSegment = useAppStore((s) => s.setSelectedSegment);

  if (segments.length === 0) {
    return (
      <div style={styles.container}>
        <div style={styles.header}>{t("segments.title")}</div>
        <div style={styles.empty}>{t("segments.empty")}</div>
      </div>
    );
  }

  return (
    <div style={styles.container}>
      <div style={styles.header}>{t("segments.title")} ({segments.length})</div>
      <div style={styles.list}>
        {segments.map((seg, i) => (
          <div
            key={seg.id}
            onClick={() =>
              setSelectedSegment(selectedSegment === seg.id ? null : seg.id)
            }
            style={{
              ...styles.item,
              ...(selectedSegment === seg.id ? styles.itemActive : {}),
            }}
          >
            <div
              style={{
                ...styles.colorDot,
                backgroundColor: SEGMENT_COLORS[i % SEGMENT_COLORS.length],
              }}
            />
            <span style={styles.itemName}>{seg.name}</span>
            <span style={styles.itemCount}>{seg.faceCount}</span>
          </div>
        ))}
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
  empty: {
    color: "#888",
    fontSize: 12,
    fontStyle: "italic",
  },
  list: {
    display: "flex",
    flexDirection: "column",
    gap: 2,
    maxHeight: 300,
    overflowY: "auto",
  },
  item: {
    display: "flex",
    alignItems: "center",
    gap: 8,
    padding: "4px 8px",
    borderRadius: 4,
    cursor: "pointer",
    color: "#ccc",
    fontSize: 13,
  },
  itemActive: {
    background: "#3a5a7a",
  },
  colorDot: {
    width: 10,
    height: 10,
    borderRadius: "50%",
    flexShrink: 0,
  },
  itemName: {
    flex: 1,
  },
  itemCount: {
    color: "#888",
    fontSize: 11,
  },
};
