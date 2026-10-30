import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import { useAppStore } from "../../store/appStore";
import { useTauriCommand } from "../../hooks/useTauriCommand";
import { useT } from "../../i18n";
import { segmentColorHex } from "../../utils/segmentPalette";
import { toggleHiddenLabels, isolateLabel, pruneHiddenLabels } from "../../utils/segmentVisibility";
import {
  buildAlgorithm,
  DEFAULT_ALGORITHM_PARAMS,
  visibleAlgorithmKinds,
  type AlgorithmKind,
  type AlgorithmParams,
} from "../../types/segment";

export function SegmentsPanel() {
  const t = useT();
  const segments = useAppStore((s) => s.segments);
  const selectedSegment = useAppStore((s) => s.selectedSegment);
  const setSelectedSegment = useAppStore((s) => s.setSelectedSegment);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const { renameSegment, mergeSegments, splitSegment, resegmentRegion, resetSegmentation } = useTauriCommand();
  const isLoaded = useAppStore((s) => s.isLoaded);
  // Issue #24: per-region visibility. View state only — hidden regions still
  // export, never enter the undo timeline, and are skipped by picking.
  const hiddenSegments = useAppStore((s) => s.hiddenSegments);
  const setHiddenSegments = useAppStore((s) => s.setHiddenSegments);

  // Eye toggle for one row, honouring the panel's two selection mechanisms:
  // Alt+click isolates the row (hides everything else); otherwise a row in
  // the merge multi-selection toggles the WHOLE selection (net toggle).
  // Every visibility change funnels through here so the spec's rule holds in
  // ONE place: hiding the viewport-selected region clears the selection —
  // the panel and the viewport must never disagree about what is on screen.
  const applyHidden = (next: Set<number>) => {
    setHiddenSegments(next);
    if (selectedSegment !== null && next.has(selectedSegment)) {
      setSelectedSegment(null);
    }
  };

  const onEyeClick = (e: React.MouseEvent, segId: number) => {
    e.stopPropagation();
    if (e.altKey) {
      applyHidden(isolateLabel(segments.map((sg) => sg.id), segId));
      setStatusMessage(t("segments.isolateDone", segId));
      return;
    }
    const group = pickedRegions.includes(segId) ? pickedRegions : [segId];
    applyHidden(toggleHiddenLabels(hiddenSegments, group));
  };

  // Ordered multi-selection (#29 unification: shared with the viewport tools).
  // `pickedRegions[0]` is the anchor and becomes
  // the surviving region — the target is never inferred from size or label,
  // because every such rule is wrong for some selection and silently produces
  // a merge into a region the user did not point at.
  //
  // Kept local rather than in the store: `selectedSegment` drives the viewport
  // highlight/dim path, and widening it to a set would put a second meaning on
  // a value the render loop reads every frame.
  // Issue #24 (feedback): lifted to the store so a click outside the model
  // (viewport empty space) can blur the merge selection.
  const pickedRegions = useAppStore((s) => s.pickedRegions);
  const setPickedRegions = useAppStore((s) => s.setPickedRegions);
  const [merging, setMerging] = useState(false);

  // Which row has its split form open, and the crease threshold for it. Kept
  // local like the edit/merge state: a split acts on one region the user points
  // at, and widening the store selection would collide with the merge multi-select
  // that already lives there.
  const [splitFor, setSplitFor] = useState<number | null>(null);
  const [splitThreshold, setSplitThreshold] = useState(30);
  const [splitting, setSplitting] = useState(false);

  // Which row has its re-segment form open, the algorithm + k chosen for it, and
  // an in-flight flag. Mirrors the split form but drives `resegmentRegion`, which
  // re-runs a full segmentation algorithm *inside* the selected region only.
  const [resegmentFor, setResegmentFor] = useState<number | null>(null);
  const [resegKind, setResegKind] = useState<AlgorithmKind>("concavity");
  const [resegK, setResegK] = useState(6);
  const [resegmenting, setResegmenting] = useState(false);
  // Experimental kinds (sdfGraphCut / concavity) are hidden behind the About
  // dialog's "show experimental" preference. `resegKind` keeps "concavity" as
  // its raw default so the flag-on path is unchanged; `effectiveResegKind`
  // degrades to curvatureKMeans (the iter-45 documented default) whenever the
  // raw kind is hidden — including "flag switched off while the form is open".
  // Render-time derivation, no effect: no dangling first frame, no extra render.
  const showExperimental = useAppStore((s) => s.showExperimental);
  const visibleKinds = useMemo(() => visibleAlgorithmKinds(showExperimental), [showExperimental]);
  const effectiveResegKind = visibleKinds.includes(resegKind) ? resegKind : "curvatureKMeans";

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

  // B2 (iteration 57): "I don't like the auto-segment, let me start over."
  // Destructive, so gate it behind a native confirm. There is intentionally no
  // undo for this (the reset itself also clears the undo stack on the backend),
  // which is why the confirm is non-negotiable.
  const onResetSegmentation = async () => {
    if (!window.confirm(t("segments.resetConfirm"))) return;
    try {
      await resetSegmentation();
    } catch {
      // error already surfaced via status message in the hook
    }
  };

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
    const live = pickedRegions.filter((id) => segments.some((s) => s.id === id));
    // Only set when something ACTUALLY left — a fresh array reference on
    // every run would loop this effect forever (React update-depth crash).
    if (live.length === pickedRegions.length) return;
    setPickedRegions(live);
  }, [segments, pickedRegions, setPickedRegions]);

  // Issue #24 (field feedback): a Ctrl-multi-selection needs a way OUT —
  // Esc clears it (blur), like every other modal-ish selection here.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (pickedRegions.length > 0) setPickedRegions([]);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [pickedRegions]);

  // Issue #24: a hidden label that no longer exists (merge / reset / undo
  // rewrote the partition) must not hide a future region reusing the label.
  useEffect(() => {
    const live = new Set(segments.map((sg) => sg.id));
    const next = pruneHiddenLabels(hiddenSegments, live);
    if (next.size !== hiddenSegments.size) setHiddenSegments(next);
  }, [segments, hiddenSegments, setHiddenSegments]);

  // The open split form must also close if its region vanishes underneath it.
  useEffect(() => {
    if (splitFor !== null && !segments.some((s) => s.id === splitFor)) {
      setSplitFor(null);
    }
  }, [segments, splitFor]);

  // Same hazard for the re-segment form: a re-run, undo, or merge can dissolve
  // the targeted region; the form must not keep pointing at a stale label.
  useEffect(() => {
    if (resegmentFor !== null && !segments.some((s) => s.id === resegmentFor)) {
      setResegmentFor(null);
    }
  }, [segments, resegmentFor]);

  const target = pickedRegions.length >= 2 ? segments.find((s) => s.id === pickedRegions[0]) : undefined;

  const toggle = (id: number, additive: boolean) => {
    if (!additive) {
      // Plain click = single highlight (selectedSegment) ONLY. It must not
      // feed pickedRegions: that set is the Ctrl-multi-selection (#29), and a
      // plain panel click landing in it popped the multi-select card for one
      // region. Re-click the same row to clear the highlight.
      setSelectedSegment(selectedSegment === id ? null : id);
      return;
    }
    // Computed from the render's own `picked` rather than inside a functional
    // updater: React may invoke an updater twice, and `setSelectedSegment` is a
    // side effect that must not ride along.
    const next = pickedRegions.includes(id) ? pickedRegions.filter((x) => x !== id) : [...pickedRegions, id];
    setPickedRegions(next);
    setSelectedSegment(next[0] ?? null);
  };

  const runMerge = async () => {
    if (!target || merging) return;
    setMerging(true);
    const moved = await mergeSegments(target.id, pickedRegions.slice(1));
    // Issue #24: the merged region keeps the TARGET label, and a hidden
    // target must not swallow the sources invisibly — merged regions start
    // visible.
    const next = new Set(hiddenSegments);
    next.delete(target.id);
    setHiddenSegments(next);
    setMerging(false);
    if (moved === null) return;
    setStatusMessage(t("segments.merged", pickedRegions.length, moved));
    setPickedRegions([target.id]);
    setSelectedSegment(target.id);
  };

  const runSplit = async (id: number) => {
    if (splitting) return;
    setSplitting(true);
    const result = await splitSegment(id, { type: "crease", thresholdDeg: splitThreshold });
    setSplitting(false);
    setSplitFor(null);
    if (!result) return;
    setStatusMessage(t("segments.splitDone", result.movedFaces));
    // The kept piece may have been re-labelled (an auto region is promoted to the
    // manual namespace on split), so reselect it rather than the now-stale id.
    setSelectedSegment(result.keptLabel);
  };

  // Human-readable label for an algorithm kind, reused from the intelligent
  // segmentation panel's i18n keys (explicit literals keep t() type-safe).
  const ALGO_LABELS: Record<AlgorithmKind, string> = {
    curvatureKMeans: t("segmentPanel.algo.curvatureKMeans"),
    shapeDiameter: t("segmentPanel.algo.shapeDiameter"),
    dihedral: t("segmentPanel.algo.dihedral"),
    sdfGraphCut: t("segmentPanel.algo.sdfGraphCut"),
    concavity: t("segmentPanel.algo.concavity"),
    convexDecomposition: t("segmentPanel.algo.convexDecomposition"),
    curveSkeleton: t("segmentPanel.algo.curveSkeleton"),
    fhGraph: t("segmentPanel.algo.fhGraph"),
  };
  const algoLabel = (k: AlgorithmKind): string => ALGO_LABELS[k];

  const runResegment = async (id: number) => {
    if (resegmenting) return;
    setResegmenting(true);
    try {
      // Start from the default algorithm params and override the one field the
      // chosen algorithm reads as "k" (block count, hull count, or angle).
      const params = JSON.parse(
        JSON.stringify(DEFAULT_ALGORITHM_PARAMS)
      ) as AlgorithmParams;
      switch (effectiveResegKind) {
        case "dihedral":
          params.dihedral.angleThreshold = resegK;
          break;
        case "shapeDiameter":
          params.shapeDiameter.k = resegK;
          break;
        case "curvatureKMeans":
          params.curvatureKMeans.k = resegK;
          break;
        case "sdfGraphCut":
          params.sdfGraphCut.k = resegK;
          break;
        case "concavity":
          params.concavity.k = resegK;
          break;
        case "convexDecomposition":
          params.convexDecomposition.maxHulls = resegK;
          break;
        case "curveSkeleton":
          params.curveSkeleton.maxHulls = resegK;
          break;
        case "fhGraph":
          // The re-segment dialog reuses its single `k` slider; map it onto the
          // FH scale (granularity) parameter.
          params.fhGraph.scale = resegK / 10;
          break;
      }
      const algorithm = buildAlgorithm(effectiveResegKind, params);
      const result = await resegmentRegion(id, algorithm);
      if (!result) return;
      setStatusMessage(t("segments.resegmentDone", result.segments.length));
      // The target label no longer exists; clear the (now stale) selection.
      setSelectedSegment(null);
    } catch {
      // error already surfaced by useTauriCommand via status message
    } finally {
      setResegmenting(false);
      setResegmentFor(null);
    }
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
      <div style={{ ...styles.header, display: "flex", alignItems: "center", justifyContent: "space-between" }}>
        <span>{t("segments.title")} ({segments.length})</span>
        <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
          {hiddenSegments.size > 0 && (
            <span style={{ color: "var(--text-3, #888888)", fontSize: 11 }}>
              {t("segments.hiddenCount", hiddenSegments.size)}
            </span>
          )}
          <button
            onClick={() =>
              applyHidden(
                hiddenSegments.size > 0
                  ? new Set<number>()
                  : new Set(segments.map((sg) => sg.id))
              )
            }
            disabled={segments.length === 0}
            title={t("segments.toggleAll")}
            style={styles.visBtn}
          >
            {hiddenSegments.size > 0 ? t("segments.showAll") : t("segments.hideAll")}
          </button>
          <button
            onClick={onResetSegmentation}
            disabled={!isLoaded || segments.length === 0}
            title={t("segments.resetHint")}
            style={styles.resetBtn}
          >
            {t("segments.reset")}
          </button>
        </div>
      </div>
      <div style={styles.list}>
        {segments.map((seg) => {
          const editing = editingId === seg.id;
          const pickIndex = pickedRegions.indexOf(seg.id);
          const splitOpen = splitFor === seg.id && !editing;
          const resegOpen = resegmentFor === seg.id && !editing;
          return (
            <Fragment key={seg.id}>
              <div
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
                  // Issue #24: a hidden row dims — the region is still in the
                  // model and still exports, it is just not on screen.
                  ...(hiddenSegments.has(seg.id) ? styles.itemHidden : {}),
                }}
              >
                <button
                  className="cym-btn"
                  onClick={(e) => onEyeClick(e, seg.id)}
                  title={t("segments.eyeHint")}
                  style={{
                    ...styles.eyeBtn,
                    ...(hiddenSegments.has(seg.id) ? styles.eyeHidden : {}),
                  }}
                >
                  {hiddenSegments.has(seg.id) ? "🚫" : "👁"}
                </button>
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
                {!editing && !splitOpen && !resegOpen && (
                  <button
                    title={t("segments.splitHint")}
                    onClick={(e) => {
                      e.stopPropagation();
                      setSplitFor(seg.id);
                    }}
                    style={styles.splitButton}
                  >
                    {t("segments.split")}
                  </button>
                )}
                {!editing && !splitOpen && !resegOpen && (
                  <button
                    title={t("segments.resegmentHint")}
                    onClick={(e) => {
                      e.stopPropagation();
                      setResegmentFor(seg.id);
                    }}
                    style={styles.resegmentButton}
                  >
                    {t("segments.resegment")}
                  </button>
                )}
              </div>
              {splitOpen && (
                <div style={styles.splitForm} onClick={(e) => e.stopPropagation()}>
                  <span style={styles.splitFormLabel}>{t("segments.splitThreshold")}</span>
                  <input
                    type="range"
                    min={5}
                    max={90}
                    value={splitThreshold}
                    onChange={(e) => setSplitThreshold(Number(e.target.value))}
                    style={{ width: 90 }}
                  />
                  <span style={styles.splitThreshVal}>{splitThreshold}°</span>
                  <button
                    onClick={() => void runSplit(seg.id)}
                    disabled={splitting}
                    style={styles.splitGo}
                  >
                    {t("segments.splitAlongCreases")}
                  </button>
                  <button onClick={() => setSplitFor(null)} style={styles.splitCancel}>
                    ×
                  </button>
                </div>
              )}
              {resegOpen && (
                <div style={styles.resegForm} onClick={(e) => e.stopPropagation()}>
                  <span style={styles.splitFormLabel}>{t("segments.resegmentAlgo")}</span>
                  <select
                    value={effectiveResegKind}
                    onChange={(e) => setResegKind(e.target.value as AlgorithmKind)}
                    style={styles.resegSelect}
                  >
                    {visibleKinds.map((k) => (
                      <option key={k} value={k}>
                        {algoLabel(k)}
                      </option>
                    ))}
                  </select>
                  <input
                    type="range"
                    min={1}
                    max={48}
                    value={resegK}
                    onChange={(e) => setResegK(Number(e.target.value))}
                    style={{ width: 80 }}
                  />
                  <span style={styles.splitThreshVal}>{resegK}</span>
                  <button
                    onClick={() => void runResegment(seg.id)}
                    disabled={resegmenting}
                    style={styles.resegGo}
                  >
                    {t("segments.resegmentRun")}
                  </button>
                  <button onClick={() => setResegmentFor(null)} style={styles.splitCancel}>
                    ×
                  </button>
                </div>
              )}
            </Fragment>
          );
        })}
      </div>
      {target && (
        <button
          onClick={() => void runMerge()}
          disabled={merging}
          style={styles.mergeButton}
        >
          {t("segments.mergeInto", pickedRegions.length, target.name)}
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
  resetBtn: {
    padding: "3px 10px",
    borderRadius: 6,
    border: "1px solid var(--border, #555)",
    background: "transparent",
    color: "var(--text-2, #ccc)",
    cursor: "pointer",
    fontSize: 12,
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
    // Capped so a rich partition (100+ regions) doesn't push the brush and
    // colour panels below the fold — those stay reachable without scrolling
    // the whole right rail (GUI audit round 2, B16).
    maxHeight: 180,
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
  // Issue #24: eye toggle + hidden-row dimming + header visibility buttons.
  eyeBtn: {
    flexShrink: 0,
    width: 20,
    padding: 0,
    border: "none",
    background: "transparent",
    cursor: "pointer",
    fontSize: 12,
    lineHeight: 1,
  },
  eyeHidden: {
    opacity: 0.5,
  },
  itemHidden: {
    opacity: 0.45,
  },
  visBtn: {
    padding: "2px 8px",
    borderRadius: 4,
    border: "1px solid var(--border, #555555)",
    background: "transparent",
    color: "var(--text-2, #bbbbbb)",
    fontSize: 11,
    cursor: "pointer",
    whiteSpace: "nowrap" as const,
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
  // Per-row "Split" affordance. Sits to the right of the face count so the rename
  // double-click and the merge Ctrl-click select stay on the main row body.
  splitButton: {
    flexShrink: 0,
    marginLeft: 6,
    padding: "1px 6px",
    borderRadius: 3,
    border: "1px solid var(--text-3, #888888)",
    background: "transparent",
    color: "var(--text-2, #bbbbbb)",
    fontSize: 11,
    cursor: "pointer",
  },
  // Sub-row shown under the target region while its split form is open. A separate
  // row (not inline in the main row) keeps the name/flex layout from shifting.
  splitForm: {
    display: "flex",
    alignItems: "center",
    gap: 6,
    padding: "4px 8px",
    marginLeft: 18,
    borderRadius: 4,
    background: "var(--bg-input, #1e1e1e)",
  },
  splitFormLabel: {
    color: "var(--text-3, #888888)",
    fontSize: 11,
  },
  splitThreshVal: {
    color: "var(--text-2, #bbbbbb)",
    fontSize: 11,
    minWidth: 28,
    textAlign: "right",
  },
  splitGo: {
    padding: "2px 8px",
    borderRadius: 3,
    border: "1px solid var(--accent, #4a9eff)",
    background: "transparent",
    color: "var(--accent, #4a9eff)",
    fontSize: 11,
    cursor: "pointer",
  },
  splitCancel: {
    padding: "2px 7px",
    borderRadius: 3,
    border: "1px solid var(--text-3, #888888)",
    background: "transparent",
    color: "var(--text-2, #bbbbbb)",
    fontSize: 12,
    lineHeight: 1,
    cursor: "pointer",
  },
  // "Re-cut" affordance — sits next to the per-row Split button.
  resegmentButton: {
    flexShrink: 0,
    marginLeft: 4,
    padding: "1px 6px",
    borderRadius: 3,
    border: "1px solid var(--accent, #4a9eff)",
    background: "transparent",
    color: "var(--accent, #4a9eff)",
    fontSize: 11,
    cursor: "pointer",
  },
  // Sub-row shown under the target region while its re-segment form is open.
  resegForm: {
    display: "flex",
    alignItems: "center",
    gap: 6,
    padding: "4px 8px",
    marginLeft: 18,
    borderRadius: 4,
    background: "var(--bg-input, #1e1e1e)",
    flexWrap: "wrap",
  },
  resegSelect: {
    background: "var(--bg-input, #1e1e1e)",
    color: "var(--text-1, #dddddd)",
    border: "1px solid var(--text-3, #888888)",
    borderRadius: 3,
    fontSize: 11,
    padding: "1px 2px",
  },
  resegGo: {
    padding: "2px 8px",
    borderRadius: 3,
    border: "1px solid var(--accent, #4a9eff)",
    background: "transparent",
    color: "var(--accent, #4a9eff)",
    fontSize: 11,
    cursor: "pointer",
  },
};
