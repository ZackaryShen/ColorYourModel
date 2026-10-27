import { useCallback, useEffect, useState } from "react";
import { useAppStore } from "../store/appStore";
import { useT } from "../i18n";
import { applyHistoryResult } from "../hooks/useHistory";
import { useTauriCommand } from "../hooks/useTauriCommand";
import { buildHistoryRows, formatHeaderBytes } from "../utils/historyPanel";
import type { HistoryEntriesResult, HistoryOpKind } from "../types/mesh";
import { log } from "../utils/logger";

/** Row icon per backend wire key (`OpKind::key()`). Fill is recorded as
 *  Paint and is indistinguishable in the entry data — the brush icon covers
 *  both, matching what the timeline actually knows. */
const KIND_ICON: Record<HistoryOpKind, string> = {
  paint: "🖌",
  segmentPaint: "🖌",
  region: "🧵",
  merge: "🔀",
  split: "✂️",
  transform: "📦",
};

/**
 * Photoshop-style history panel (issue #22): one row per applied operation
 * (plus the pristine state), a marker on the live state, the redo branch
 * greyed below it. Clicking any row jumps there in ONE backend call
 * (`history_jump`); the result rides the same applyHistoryResult bridge as
 * undo/redo, whose flag update bumps `historyVersion` — which refetches this
 * panel. New operations truncate the redo branch on the backend already
 * (`drop_redo`); the next fetch simply shows fewer rows.
 */
export function HistoryPanel() {
  const t = useT();
  const meshData = useAppStore((s) => s.meshData);
  const historyVersion = useAppStore((s) => s.historyVersion);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const { historyEntries, historyJump } = useTauriCommand();
  const [data, setData] = useState<HistoryEntriesResult | null>(null);
  const [jumping, setJumping] = useState(false);

  // Refetch whenever the timeline may have moved (version bump) or a model
  // (re)loaded (panel state is per model, resetting on import like the
  // timeline itself). Identified deps only: the wrappers change identity
  // every render by design.
  const fetchEntries = useCallback(async () => {
    const res = await historyEntries();
    if (res) setData(res);
  }, [historyEntries]);

  useEffect(() => {
    void fetchEntries();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [historyVersion, meshData]);

  const onJump = async (depth: number) => {
    if (jumping || depth === (data?.current ?? 0)) return;
    setJumping(true);
    try {
      const result = await historyJump(depth);
      if (result) {
        applyHistoryResult(result); // flag update bumps historyVersion → refetch
        setStatusMessage(t("history.jumped", depth));
      }
    } catch (e) {
      log.error("HistoryPanel", "jump failed", { depth, error: String(e) });
      setStatusMessage(`${t("history.jumpFailed")}: ${e}`);
    } finally {
      setJumping(false);
    }
  };

  const rows = buildHistoryRows(data?.entries ?? [], data?.current ?? 0);

  return (
    <div style={styles.panel} role="listbox" aria-label={t("history.title")}>
      <div style={styles.header}>
        <span style={styles.title}>{t("history.title")}</span>
        {data && (
          <span style={styles.headerMeta}>
            {t("history.header", data.current, data.maxDepth, formatHeaderBytes(data.bytes))}
          </span>
        )}
      </div>
      <div style={styles.list}>
        {rows.map((row) => (
          <button
            key={row.depth}
            className="cym-btn"
            style={{
              ...styles.row,
              ...(row.isCurrent ? styles.rowCurrent : {}),
              ...(row.isRedo ? styles.rowRedo : {}),
            }}
            onClick={() => void onJump(row.depth)}
            disabled={row.isCurrent || jumping}
            title={row.isRedo ? t("history.redoHint") : t("history.undoHint")}
            role="option"
            aria-selected={row.isCurrent}
          >
            <span style={styles.icon}>{row.kind ? KIND_ICON[row.kind] ?? "•" : "🏠"}</span>
            <span style={styles.rowLabel}>
              {row.kind ? t(`history.op.${row.kind}`) : t("history.pristine")}
            </span>
            {row.faceCount !== null && (
              <span style={styles.faces}>{t("history.faces", row.faceCount.toLocaleString())}</span>
            )}
          </button>
        ))}
        {data && data.entries.length === 0 && (
          <div style={styles.empty}>{t("history.empty")}</div>
        )}
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  panel: {
    width: 236,
    display: "flex",
    flexDirection: "column",
    overflow: "hidden",
    borderLeft: "1px solid var(--border, #444)",
    background: "var(--bg-root, #1e1e1e)",
  },
  header: {
    padding: "8px 10px 6px",
    borderBottom: "1px solid var(--border, #444)",
    display: "flex",
    flexDirection: "column",
    gap: 2,
  },
  title: {
    fontSize: 13,
    fontWeight: 600,
    color: "var(--text-1, #eee)",
  },
  headerMeta: {
    fontSize: 11,
    color: "var(--text-2, #999)",
  },
  list: {
    flex: 1,
    overflowY: "auto",
    padding: 6,
    display: "flex",
    flexDirection: "column",
    gap: 2,
  },
  row: {
    display: "flex",
    alignItems: "center",
    gap: 6,
    width: "100%",
    padding: "5px 8px",
    border: "1px solid transparent",
    background: "transparent",
    color: "var(--text-1, #ddd)",
    borderRadius: 6,
    cursor: "pointer",
    fontSize: 12,
    textAlign: "left",
  },
  rowCurrent: {
    background: "var(--bg-elevated, #3a3a3a)",
    borderColor: "var(--accent, #4a9eff)",
    fontWeight: 600,
    cursor: "default",
  },
  rowRedo: {
    opacity: 0.45,
  },
  icon: {
    flexShrink: 0,
    width: 18,
    textAlign: "center",
  },
  rowLabel: {
    flex: 1,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  faces: {
    flexShrink: 0,
    fontSize: 10,
    color: "var(--text-2, #999)",
  },
  empty: {
    padding: 10,
    fontSize: 12,
    color: "var(--text-2, #999)",
  },
};
