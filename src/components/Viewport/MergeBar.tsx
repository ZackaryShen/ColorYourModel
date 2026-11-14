import { useEffect } from "react";
import { useAppStore } from "../../store/appStore";
import { useSegmentMerge } from "../../hooks/useSegmentMerge";
import { useT } from "../../i18n";
import { displaySegmentName } from "../../utils/segmentLabel";

/**
 * Floating merge bar (issue #16): bottom-centre HTML overlay over the canvas,
 * shown while the shared merge multi-select holds two or more regions. The
 * button label reuses the panel's "merge N into X" string so both entry points
 * promise the exact same outcome — anchor comes from `mergePicked[0]`, never
 * inferred.
 */
export function MergeBar() {
  const t = useT();
  const segments = useAppStore((s) => s.segments);
  const setMergePicked = useAppStore((s) => s.setMergePicked);
  const { picked, merging, mergePicked, clear } = useSegmentMerge();

  // Same staleness rule as SegmentsPanel's prune effect: a merge, an undo or a
  // re-run can dissolve a picked region, and a dead label must never reach the
  // backend. Deliberately duplicated (not a shared hook) — one line of policy.
  useEffect(() => {
    const live = picked.filter((id) => segments.some((s) => s.id === id));
    if (live.length !== picked.length) setMergePicked(live);
  }, [segments, picked, setMergePicked]);

  if (picked.length < 2) return null;
  const anchor = segments.find((s) => s.id === picked[0]);

  return (
    <div style={mergeBarStyles.wrap}>
      <div style={mergeBarStyles.bar}>
        <button
          className="cym-btn"
          style={mergeBarStyles.merge}
          disabled={merging || !anchor}
          onClick={() => void mergePicked()}
        >
          {t("segments.mergeInto", picked.length, anchor ? displaySegmentName(anchor.name, t) : "")}
        </button>
        <button
          className="cym-btn"
          style={mergeBarStyles.clear}
          title={t("merge.clearTitle")}
          onClick={clear}
        >
          ×
        </button>
      </div>
    </div>
  );
}

const mergeBarStyles: Record<string, React.CSSProperties> = {
  wrap: {
    position: "absolute",
    bottom: 16,
    left: "50%",
    transform: "translateX(-50%)",
    zIndex: 20,
  },
  bar: {
    display: "flex",
    alignItems: "center",
    gap: 8,
    padding: "6px 8px",
    background: "var(--toast-bg, rgba(34,40,48,0.95))",
    border: "1px solid var(--accent, #4a9eff)",
    borderRadius: 8,
    boxShadow: "0 6px 24px var(--shadow, rgba(0,0,0,0.3))",
  },
  merge: {
    padding: "4px 10px",
    border: "1px solid var(--accent-border, #2b6cb0)",
    background: "var(--accent, #3a5a7a)",
    color: "#fff",
    borderRadius: 6,
    cursor: "pointer",
    fontSize: 12,
    fontWeight: 600,
    whiteSpace: "nowrap",
  },
  clear: {
    padding: "2px 8px",
    border: "1px solid var(--border, #555)",
    background: "transparent",
    color: "var(--text-2, #aaa)",
    borderRadius: 6,
    fontSize: 13,
    lineHeight: 1,
    cursor: "pointer",
  },
};
