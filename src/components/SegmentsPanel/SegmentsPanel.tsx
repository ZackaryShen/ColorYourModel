import { useEffect, useRef, useState } from "react";
import { useAppStore } from "../../store/appStore";
import { useTauriCommand } from "../../hooks/useTauriCommand";
import { useT } from "../../i18n";
import { segmentColorHex } from "../../utils/segmentPalette";

export function SegmentsPanel() {
  const t = useT();
  const segments = useAppStore((s) => s.segments);
  const selectedSegment = useAppStore((s) => s.selectedSegment);
  const setSelectedSegment = useAppStore((s) => s.setSelectedSegment);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const { renameSegment, mergeSegments } = useTauriCommand();

  // Ordered multi-selection for merging. `picked[0]` is the anchor and becomes
  // the surviving region — the target is never inferred from size or label,
  // because every such rule is wrong for some selection and silently produces
  // a merge into a region the user did not point at.
  //
  // Kept local rather than in the store: `selectedSegment` drives the viewport
  // highlight/dim path, and widening it to a set would put a second meaning on
  // a value the render loop reads every frame.
  const [picked, setPicked] = useState<number[]>([]);
  const [merging, setMerging] = useState(false);

  // Which row is in edit mode, and the text being typed. Held here rather than
  // per-row so only one row can ever be open: two open editors would both
  // commit on blur and race each other's backend calls.
  const [editingId, setEditingId] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const inputRef = useRef<HTMLInputElement | null>(null);
  // Guards the blur handler. Committing on blur is what makes "click away to
  // save" work, but Enter and Escape also remove focus, so without this the
  // commit would run a second time against a row that is already closed.
  const closingRef = useRef(false);

  useEffect(() => {
    if (editingId !== null) {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
  }, [editingId]);

  // A row can disappear underneath an open editor (undo dissolves a lasso
  // region, a re-run renumbers everything). Leaving the editor open would let
  // the next commit target a label that no longer means the same thing.
  useEffect(() => {
    if (editingId !== null && !segments.some((s) => s.id === editingId)) {
      setEditingId(null);
    }
  }, [segments, editingId]);

  // Same hazard for the merge selection: a merge, an undo or a re-run can
  // dissolve a picked region, and sending its label to the backend afterwards
  // fails the existence check — or worse, hits a label that has been reissued.
  useEffect(() => {
    setPicked((prev) => {
      const live = prev.filter((id) => segments.some((s) => s.id === id));
      return live.length === prev.length ? prev : live;
    });
  }, [segments]);

  const target = picked.length >= 2 ? segments.find((s) => s.id === picked[0]) : undefined;

  const toggle = (id: number, additive: boolean) => {
    if (!additive) {
      const single = picked.length === 1 && picked[0] === id;
      setPicked(single ? [] : [id]);
      setSelectedSegment(single ? null : id);
      return;
    }
    // Computed from the render's own `picked` rather than inside a functional
    // updater: React may invoke an updater twice, and `setSelectedSegment` is a
    // side effect that must not ride along.
    const next = picked.includes(id) ? picked.filter((x) => x !== id) : [...picked, id];
    setPicked(next);
    setSelectedSegment(next[0] ?? null);
  };

  const runMerge = async () => {
    if (!target || merging) return;
    setMerging(true);
    const moved = await mergeSegments(target.id, picked.slice(1));
    setMerging(false);
    if (moved === null) return;
    setStatusMessage(t("segments.merged", picked.length, moved));
    setPicked([target.id]);
    setSelectedSegment(target.id);
  };

  const beginEdit = (id: number, current: string) => {
    closingRef.current = false;
    setDraft(current);
    setEditingId(id);
  };

  const commit = async (id: number) => {
    if (closingRef.current) return;
    closingRef.current = true;
    setEditingId(null);
    await renameSegment(id, draft);
  };

  const cancel = () => {
    closingRef.current = true;
    setEditingId(null);
  };

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
        {segments.map((seg) => {
          const editing = editingId === seg.id;
          const pickIndex = picked.indexOf(seg.id);
          return (
            <div
              key={seg.id}
              onClick={(e) => {
                if (editing) return;
                toggle(seg.id, e.ctrlKey || e.metaKey);
              }}
              onDoubleClick={() => beginEdit(seg.id, seg.name)}
              title={t("segments.renameHint")}
              style={{
                ...styles.item,
                ...(selectedSegment === seg.id ? styles.itemActive : {}),
                ...(pickIndex >= 0 ? styles.itemPicked : {}),
              }}
            >
              <div
                style={{
                  ...styles.colorDot,
                  // Keyed by label, matching what the segment view paints. The
                  // list index would drift the moment a label is missing from
                  // the 0..N run, which is the normal case.
                  backgroundColor: segmentColorHex(seg.id),
                }}
              />
              {editing ? (
                <input
                  ref={inputRef}
                  value={draft}
                  onChange={(e) => setDraft(e.target.value)}
                  onClick={(e) => e.stopPropagation()}
                  onBlur={() => void commit(seg.id)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      void commit(seg.id);
                    } else if (e.key === "Escape") {
                      e.preventDefault();
                      cancel();
                    }
                    // Painting shortcuts live on window; a keystroke meant for
                    // this field must not also switch tools.
                    e.stopPropagation();
                  }}
                  maxLength={64}
                  style={styles.itemInput}
                />
              ) : (
                <span style={styles.itemName}>{seg.name}</span>
              )}
              <span style={styles.itemCount}>{seg.faceCount}</span>
            </div>
          );
        })}
      </div>
      {target && (
        <button
          onClick={() => void runMerge()}
          disabled={merging}
          style={styles.mergeButton}
        >
          {t("segments.mergeInto", picked.length, target.name)}
        </button>
      )}
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
  empty: {
    color: "var(--text-3, #888888)",
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
    color: "var(--text-1, #cccccc)",
    fontSize: 13,
  },
  itemActive: {
    background: "var(--bg-active, #3a5a7a)",
  },
  // Distinct from `itemActive`: a row can be the viewport's selected segment,
  // part of the merge selection, or both, and the two states have to stay
  // readable when they overlap.
  itemPicked: {
    boxShadow: "inset 2px 0 0 var(--accent, #4a9eff)",
  },
  colorDot: {
    width: 10,
    height: 10,
    borderRadius: "50%",
    flexShrink: 0,
  },
  itemName: {
    flex: 1,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  itemInput: {
    flex: 1,
    minWidth: 0,
    padding: "1px 4px",
    border: "1px solid var(--accent, #4a9eff)",
    borderRadius: 3,
    background: "var(--bg-input, #1e1e1e)",
    color: "var(--text-1, #dddddd)",
    font: "inherit",
    fontSize: 13,
  },
  itemCount: {
    color: "var(--text-3, #888888)",
    fontSize: 11,
  },
  mergeButton: {
    marginTop: 8,
    width: "100%",
    padding: "5px 8px",
    borderRadius: 4,
    border: "1px solid var(--accent, #4a9eff)",
    background: "transparent",
    color: "var(--accent, #4a9eff)",
    fontSize: 12,
    cursor: "pointer",
  },
};
