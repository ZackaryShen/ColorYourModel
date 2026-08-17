import { useEffect, useState } from "react";
import { useAppStore } from "../store/appStore";
import { PaintTool } from "../types/mesh";
import { useTauriCommand } from "../hooks/useTauriCommand";
import { useT } from "../i18n";
import { log } from "../utils/logger";

/// Seeded-watershed control panel (iteration 50). Shown while the Seed tool is
/// active. The user places seed points on the mesh (handled in Viewport), and
/// this panel tunes the barrier angle, toggles the optimizer, and triggers grow.
export function SeedPanel() {
  const t = useT();
  const seedPoints = useAppStore((s) => s.seedPoints);
  const clearSeedPoints = useAppStore((s) => s.clearSeedPoints);
  const seedEraseMode = useAppStore((s) => s.seedEraseMode);
  const setSeedEraseMode = useAppStore((s) => s.setSeedEraseMode);
  const suggestedSeeds = useAppStore((s) => s.suggestedSeeds);
  const setSuggestedSeeds = useAppStore((s) => s.setSuggestedSeeds);
  const clearSuggestedSeeds = useAppStore((s) => s.clearSuggestedSeeds);
  const planarRegions = useAppStore((s) => s.planarRegions);
  const setPlanarRegions = useAppStore((s) => s.setPlanarRegions);
  const clearPlanarRegions = useAppStore((s) => s.clearPlanarRegions);
  const planarRegionsVisible = useAppStore((s) => s.planarRegionsVisible);
  const togglePlanarRegionsVisible = useAppStore((s) => s.togglePlanarRegionsVisible);
  const multiviewRegions = useAppStore((s) => s.multiviewRegions);
  const setMultiviewRegions = useAppStore((s) => s.setMultiviewRegions);
  const clearMultiviewRegions = useAppStore((s) => s.clearMultiviewRegions);
  const multiviewRegionsVisible = useAppStore((s) => s.multiviewRegionsVisible);
  const toggleMultiviewRegionsVisible = useAppStore((s) => s.toggleMultiviewRegionsVisible);
  const crossSectionRegions = useAppStore((s) => s.crossSectionRegions);
  const setCrossSectionRegions = useAppStore((s) => s.setCrossSectionRegions);
  const clearCrossSectionRegions = useAppStore((s) => s.clearCrossSectionRegions);
  const crossSectionRegionsVisible = useAppStore((s) => s.crossSectionRegionsVisible);
  const toggleCrossSectionRegionsVisible = useAppStore((s) => s.toggleCrossSectionRegionsVisible);
  const eyeRegions = useAppStore((s) => s.eyeRegions);
  const setEyeRegions = useAppStore((s) => s.setEyeRegions);
  const clearEyeRegions = useAppStore((s) => s.clearEyeRegions);
  const eyeRegionsVisible = useAppStore((s) => s.eyeRegionsVisible);
  const toggleEyeRegionsVisible = useAppStore((s) => s.toggleEyeRegionsVisible);
  const selectedSegment = useAppStore((s) => s.selectedSegment);
  const setOnlyVisible = useAppStore((s) => s.setOnlyVisible);
  const meshData = useAppStore((s) => s.meshData);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const setActiveTool = useAppStore((s) => s.setActiveTool);
  const { seedGrow, recommendSeeds, detectPlanarRegions, detectMultiViewRegions, detectCrossSectionRegions, detectEyeRegions, fuseSegmentation, resetSegmentation } = useTauriCommand();

  // Iteration 66: give the panel an obvious "I'm done here" exit affordance.
  // The × button on the title row and the Esc key both switch back to View —
  // same semantics as IntelligentSegmentPanel uses for its modal. Seed / ghost /
  // algorithm region state is left in the store so re-entering the Seed tool
  // restores the in-progress workflow.
  const closePanel = () => {
    log.info("SeedPanel", "closePanel → View");
    setActiveTool(PaintTool.View);
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        log.info("SeedPanel", "Esc pressed → View");
        setActiveTool(PaintTool.View);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setActiveTool]);

  const [barrierDeg, setBarrierDeg] = useState(45);
  const [optimizer, setOptimizer] = useState(false);
  const [growing, setGrowing] = useState(false);
  const [recommending, setRecommending] = useState(false);
  const [suggestCount, setSuggestCount] = useState(12);
  const [curvWeight, setCurvWeight] = useState(1.0);
  const [concWeight, setConcWeight] = useState(1.0);
  const [detecting, setDetecting] = useState(false);
  const [detectingEye, setDetectingEye] = useState(false);
  const [fusing, setFusing] = useState(false);
  const [resetting, setResetting] = useState(false);

  const onRecommend = async () => {
    setRecommending(true);
    log.info("SeedPanel", "onRecommend click", {
      suggestCount,
      curvWeight,
      concWeight,
    });
    try {
      const seeds = await recommendSeeds(suggestCount, curvWeight, concWeight);
      log.info("SeedPanel", "onRecommend received", { count: seeds.length });
      setSuggestedSeeds(seeds);
    } catch {
      // error already surfaced via status message in recommendSeeds
    } finally {
      setRecommending(false);
    }
  };

  // Layer 1 (docs/09): detect the mesh's continuous planar regions and feed
  // them in as advisory seed suggestions. Each region's representative seed is
  // pushed into the ghost-suggestion set (reuse iter58-64 accept path), and its
  // boundary outline is stored for the Viewport to draw. Thresholds follow the
  // cited defaults (angle 15° ≈ π/12, dist M/30); min region size scales with
  // mesh resolution so a 500k-face model doesn't emit 50 tiny patches.
  const onDetectPlanar = async () => {
    setDetecting(true);
    log.info("SeedPanel", "onDetectPlanar click");
    const faceCount = meshData?.faceCount ?? 0;
    const minFaces = Math.max(2, Math.floor(faceCount / 500));
    try {
      const regions = await detectPlanarRegions(15, 1 / 30, minFaces);
      log.info("SeedPanel", "onDetectPlanar received", { regions: regions.length });
      setPlanarRegions(regions);
      setSuggestedSeeds(regions.map((r) => r.seed));
    } catch {
      // error already surfaced via status message in detectPlanarRegions
    } finally {
      setDetecting(false);
    }
  };

  // Layer 3 (docs/09): MultiView 3→2→3. Project the mesh from many views, grow
  // 2D-connected regions per view, back-project and cut a weighted match graph,
  // then feed each consensus cluster in as an advisory seed suggestion. This is
  // a *second opinion* evidence channel distinct from Layer 1 (planar): it may
  // agree or disagree; both are offered, never silently overriding. Thresholds:
  // 12 views, angle 20° (a touch looser than Layer 1's 15° because the per-view
  // projection already supplies the spatial-contact constraint), min region size
  // scales with mesh resolution, match_threshold 1 (agreed in ≥1 view).
  const onDetectMultiView = async () => {
    setDetecting(true);
    log.info("SeedPanel", "onDetectMultiView click");
    const faceCount = meshData?.faceCount ?? 0;
    const minFaces = Math.max(2, Math.floor(faceCount / 500));
    try {
      const regions = await detectMultiViewRegions(12, 20, minFaces, 1);
      log.info("SeedPanel", "onDetectMultiView received", { regions: regions.length });
      setMultiviewRegions(regions);
      setSuggestedSeeds(regions.map((r) => r.seed));
    } catch {
      // error already surfaced via status message in detectMultiViewRegions
    } finally {
      setDetecting(false);
    }
  };

  // Layer 2 (docs/09): cross-section / ray-marching feature detection. Marches a
  // plane along each principal axis and reports the feature cross-sections
  // (where the cross-sectional profile changes sharply). This is *visual-only
  // evidence*: the green contours are drawn over the model so the user can SEE
  // where the shape steps/features are. It does NOT inject seeds into
  // `suggestedSeeds` / `seed_grow` — a slice is a plane, not a face, and Layer 2
  // is explicitly "不裁决" (not a verdict) in docs/09.
  const onDetectCrossSection = async () => {
    setDetecting(true);
    log.info("SeedPanel", "onDetectCrossSection click");
    try {
      const regions = await detectCrossSectionRegions(24, 0.5);
      log.info("SeedPanel", "onDetectCrossSection received", { regions: regions.length });
      setCrossSectionRegions(regions);
    } catch {
      // error already surfaced via status message in detectCrossSectionRegions
    } finally {
      setDetecting(false);
    }
  };

  // Layer 5 (docs/10): eye-region semantic decomposition. The ROI is the set of
  // faces belonging to the CURRENTLY SELECTED partition — the user first boxes
  // an eye area with any algorithm or the lasso, selects that partition, then
  // clicks here. We derive the ROI faces from `segmentLabels` (the per-face
  // partition id array) intersected with `selectedSegment`; no extra backend
  // round-trip needed. The eye detector is read-only, so its regions are stored
  // purely for the Viewport overlay (like Layer 1/2/3).
  const onDetectEye = async () => {
    setDetectingEye(true);
    log.info("SeedPanel", "onDetectEye click", { selectedSegment });
    const md = meshData;
    if (selectedSegment === null || !md || !md.segmentLabels) {
      setStatusMessage(t("seed.eyeNeedSegment"));
      setDetectingEye(false);
      return;
    }
    const roiFaces: number[] = [];
    const labels = md.segmentLabels;
    for (let i = 0; i < labels.length; i++) {
      if (labels[i] === selectedSegment) roiFaces.push(i);
    }
    if (roiFaces.length < 1) {
      setStatusMessage(t("seed.eyeNeedFaces"));
      setDetectingEye(false);
      return;
    }
    try {
      const regions = await detectEyeRegions(roiFaces);
      log.info("SeedPanel", "onDetectEye received", { regions: regions.length });
      setEyeRegions(regions);
    } catch {
      // error already surfaced via status message in detectEyeRegions
    } finally {
      setDetectingEye(false);
    }
  };

  // Layer 4 (docs/09): fuse Layer 1 planar + Layer 3 MultiView region
  // *membership* into one partition via edge-level majority vote, then commit
  // it — the fix for "the algorithms sketch useful regions but the real
  // partition is still just seeds". The backend runs both detectors with the
  // panel defaults and fuses their face sets; only cutThreshold is exposed
  // (1 = a cut must outvote keep; ties merge to suppress over-splitting).
  const onFuse = async () => {
    setFusing(true);
    log.info("SeedPanel", "onFuse click");
    try {
      const result = await fuseSegmentation(1, 0);
      log.info("SeedPanel", "onFuse done", { segments: result.segments.length });
    } catch {
      // error already surfaced via status message in fuseSegmentation
    } finally {
      setFusing(false);
    }
  };

  // "Clear everything" — 之前的 × 按钮只清某层,分区面着色依旧存在
  // (meshData.segmentLabels 来自 manual label / fuse),所以用户一直没法清空。
  // 后端 reset_segmentation 已就位(useTauriCommand.resetSegmentation):
  // 把 segmentLabels 全置 0、segments 清空、faceColors 还原默认,history
  // 也清空。前端顺手把算法 regions + 种子全部清掉,视口立刻恢复干净状态。
  const onResetAll = async () => {
    if (!window.confirm(t("segments.resetConfirm"))) return;
    setResetting(true);
    log.info("SeedPanel", "onResetAll click");
    try {
      await resetSegmentation();
      // 视口残留清理:前端持有的算法层/种子瞬态数据都清掉,BoundaryLines 立刻消失
      clearPlanarRegions();
      clearMultiviewRegions();
      clearCrossSectionRegions();
      clearEyeRegions();
      clearSeedPoints();
      clearSuggestedSeeds();
      setSeedEraseMode(false);
      setStatusMessage("✅ 已清空分区（恢复导入时的干净状态）");
    } catch {
      // status already surfaced via useTauriCommand
    } finally {
      setResetting(false);
    }
  };

  const onGrow = async () => {
    log.info("SeedPanel", "onGrow click", {
      seedPoints: seedPoints.length,
      suggested: suggestedSeeds.length,
      barrierDeg,
      optimizer,
    });
    // Iteration 60+63: the user may have only clicked "建议种子" (which fills
    // `suggestedSeeds`) and never clicked the model to accept them into
    // `seedPoints`. Grow directly from the suggestions so "建议种子 → 生成"
    // works in one shot. Previously onGrow early-returned on
    // `seedPoints.length === 0`, silently doing nothing.
    //
    // Iteration 63: do NOT drop suggestions once any manual seed exists.
    // The user's workflow is "推荐先生成一次了，我再手动微调" — after the AI
    // generates once, the ghost markers must still participate when the user
    // adds more manual seeds and re-grows. They are two views of the same
    // working set. To grow with only manual seeds, the user explicitly clicks
    // "Clear suggestions" first.
    const seeds = [...seedPoints, ...suggestedSeeds];
    if (seeds.length === 0) {
      setStatusMessage(t("seed.needOne"));
      return;
    }
    setGrowing(true);
    setStatusMessage(
      `🌱 准备生长（${seeds.length} 个种子，barrier=${barrierDeg}°，optimizer=${optimizer}）…`,
    );
    try {
      const result = await seedGrow(seeds, barrierDeg, optimizer);
      log.info("SeedPanel", "onGrow done", {
        segments: result.segments.length,
        movedFaces: result.segmentLabels.length,
      });
      setStatusMessage(t("seed.done", result.segments.length));
      // Iteration 61: do NOT clear ghost suggestions on grow success. The
      // user's workflow is "推荐先生成一次了，我再手动微调" — after the AI
      // generates once, they want the ghost markers to remain visible as
      // reference so they can keep adding manual seeds (or click more
      // suggestions) and grow again. Clearing was hiding the very reference
      // markers the user needed to fine-tune. They can be cleared explicitly
      // via the "Clear suggestions" button.
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      log.error("SeedPanel", "seedGrow failed", { error: msg });
      setStatusMessage(`🌱 种子分区失败：${msg}`);
    } finally {
      setGrowing(false);
    }
  };

  return (
    <div style={styles.panel}>
      <div style={styles.titleRow}>
        <span style={styles.title}>🌱 {t("tool.seed")}</span>
        <button
          onClick={closePanel}
          style={styles.closeBtn}
          title={t("seed.closeTitle")}
          aria-label={t("seed.closeTitle")}
        >
          ×
        </button>
      </div>
      <div style={styles.hint}>
        {seedEraseMode
          ? t("seed.eraseHint")
          : eyeRegions.length > 0
            ? t("seed.eyeHint")
            : crossSectionRegions.length > 0
              ? t("seed.crossSectionHint")
              : multiviewRegions.length > 0
                ? t("seed.multiviewHint")
                : planarRegions.length > 0
                  ? t("seed.planarHint")
                  : suggestedSeeds.length > 0
                    ? t("seed.suggestHint")
                    : t("seed.hint")}
      </div>

      <div style={styles.row}>
        <span style={styles.label}>{t("seed.count", seedPoints.length)}</span>
        <button
          onClick={() => setSeedEraseMode(!seedEraseMode)}
          style={{
            ...styles.clear,
            ...(seedEraseMode ? { borderColor: "#ff4d4f", color: "#ff4d4f" } : {}),
          }}
        >
          🩹 {t("seed.eraseMode")}
        </button>
      </div>

      <div style={styles.row}>
        <span style={styles.label}>{t("seed.suggestCount")}</span>
        <input
          type="range"
          min={2}
          max={40}
          value={suggestCount}
          onChange={(e) => setSuggestCount(Number(e.target.value))}
          style={{ flex: 1 }}
        />
        <span style={styles.val}>{suggestCount}</span>
      </div>

      <div style={styles.row}>
        <span style={styles.label} title={t("seed.weightCurvHint")}>
          {t("seed.weightCurv")}
        </span>
        <input
          type="range"
          min={0}
          max={2}
          step={0.1}
          value={curvWeight}
          onChange={(e) => setCurvWeight(Number(e.target.value))}
          style={{ flex: 1 }}
        />
        <span style={styles.val}>{curvWeight.toFixed(1)}</span>
      </div>
      <div style={styles.row}>
        <span style={styles.label} title={t("seed.weightConcHint")}>
          {t("seed.weightConc")}
        </span>
        <input
          type="range"
          min={0}
          max={2}
          step={0.1}
          value={concWeight}
          onChange={(e) => setConcWeight(Number(e.target.value))}
          style={{ flex: 1 }}
        />
        <span style={styles.val}>{concWeight.toFixed(1)}</span>
      </div>

      <div style={styles.row}>
        <button onClick={onRecommend} disabled={recommending} style={styles.grow}>
          {recommending ? "…" : t("seed.suggest")}
        </button>
        <button
          onClick={clearSuggestedSeeds}
          disabled={suggestedSeeds.length === 0}
          style={styles.clear}
        >
          {t("seed.clearSuggest", suggestedSeeds.length)}
        </button>
      </div>

      <div style={styles.row}>
        <button onClick={onDetectPlanar} disabled={detecting} style={styles.grow}>
          {detecting ? "…" : t("seed.planar")}
        </button>
        <button
          onClick={togglePlanarRegionsVisible}
          disabled={planarRegions.length === 0}
          style={{
            ...styles.toggle,
            background: planarRegionsVisible ? "#1e3a5f" : "#2a2a2a",
            color: planarRegionsVisible ? "#22d3ee" : "#888",
          }}
          title={t(planarRegionsVisible ? "seed.hide" : "seed.show")}
        >
          {planarRegionsVisible ? "👁" : "🚫"} {planarRegions.length}
        </button>
        <button
          onClick={clearPlanarRegions}
          disabled={planarRegions.length === 0}
          style={styles.clear}
          title={t("seed.clearPlanarTitle")}
        >
          ×
        </button>
      </div>

      <div style={styles.row}>
        <button onClick={onDetectMultiView} disabled={detecting} style={styles.grow}>
          {detecting ? "…" : t("seed.multiview")}
        </button>
        <button
          onClick={toggleMultiviewRegionsVisible}
          disabled={multiviewRegions.length === 0}
          style={{
            ...styles.toggle,
            background: multiviewRegionsVisible ? "#5f3a1e" : "#2a2a2a",
            color: multiviewRegionsVisible ? "#fb923c" : "#888",
          }}
          title={t(multiviewRegionsVisible ? "seed.hide" : "seed.show")}
        >
          {multiviewRegionsVisible ? "👁" : "🚫"} {multiviewRegions.length}
        </button>
        <button
          onClick={clearMultiviewRegions}
          disabled={multiviewRegions.length === 0}
          style={styles.clear}
          title={t("seed.clearMultiviewTitle")}
        >
          ×
        </button>
      </div>

      <div style={styles.row}>
        <button onClick={onDetectCrossSection} disabled={detecting} style={styles.grow}>
          {detecting ? "…" : t("seed.crossSection")}
        </button>
        <button
          onClick={toggleCrossSectionRegionsVisible}
          disabled={crossSectionRegions.length === 0}
          style={{
            ...styles.toggle,
            background: crossSectionRegionsVisible ? "#3a5f1e" : "#2a2a2a",
            color: crossSectionRegionsVisible ? "#84cc16" : "#888",
          }}
          title={t(crossSectionRegionsVisible ? "seed.hide" : "seed.show")}
        >
          {crossSectionRegionsVisible ? "👁" : "🚫"} {crossSectionRegions.length}
        </button>
        <button
          onClick={clearCrossSectionRegions}
          disabled={crossSectionRegions.length === 0}
          style={styles.clear}
          title={t("seed.clearCrossSectionTitle")}
        >
          ×
        </button>
      </div>

      <div style={styles.row}>
        <button onClick={onDetectEye} disabled={detectingEye} style={styles.grow}>
          {detectingEye ? "…" : t("seed.eye")}
        </button>
        <button
          onClick={toggleEyeRegionsVisible}
          disabled={eyeRegions.length === 0}
          style={{
            ...styles.toggle,
            background: eyeRegionsVisible ? "#3a1e5f" : "#2a2a2a",
            color: eyeRegionsVisible ? "#c084fc" : "#888",
          }}
          title={t(eyeRegionsVisible ? "seed.hide" : "seed.show")}
        >
          {eyeRegionsVisible ? "👁" : "🚫"} {eyeRegions.length}
        </button>
        <button
          onClick={clearEyeRegions}
          disabled={eyeRegions.length === 0}
          style={styles.clear}
          title={t("seed.eyeTitle")}
        >
          ×
        </button>
      </div>

      {/* Solo / all toggles — quick way to show only one layer */}
      <div style={styles.row}>
        <button
          onClick={() => setOnlyVisible(null)}
          style={styles.miniBtn}
          title={t("seed.showAll")}
        >
          {t("seed.showAll")}
        </button>
        <button
          onClick={() => setOnlyVisible("planar")}
          disabled={planarRegions.length === 0}
          style={{ ...styles.miniBtn, color: "#22d3ee" }}
          title={t("seed.soloPlanar")}
        >
          🟦
        </button>
        <button
          onClick={() => setOnlyVisible("multiview")}
          disabled={multiviewRegions.length === 0}
          style={{ ...styles.miniBtn, color: "#fb923c" }}
          title={t("seed.soloMultiview")}
        >
          👁
        </button>
        <button
          onClick={() => setOnlyVisible("crosssection")}
          disabled={crossSectionRegions.length === 0}
          style={{ ...styles.miniBtn, color: "#84cc16" }}
          title={t("seed.soloCrossSection")}
        >
          ✂️
        </button>
        <button
          onClick={() => setOnlyVisible("eye")}
          disabled={eyeRegions.length === 0}
          style={{ ...styles.miniBtn, color: "#c084fc" }}
          title={t("seed.soloEye")}
        >
          👁
        </button>
      </div>

      {/* Layer 4 fusion — one click turns the algorithm regions into the
          actual partition, no seed placement required. */}
      <div style={styles.row}>
        <button onClick={onFuse} disabled={fusing} style={styles.fuse} title={t("seed.fuseTitle")}>
          {fusing ? "…" : t("seed.fuse")}
        </button>
      </div>

      {/* Reset partition — wipes meshData.segmentLabels + faceColors + history
          and clears every algorithm overlay. Indispensable since the previous
          "× buttons" only cleared their own data slice. */}
      <div style={styles.row}>
        <button
          onClick={onResetAll}
          disabled={resetting}
          style={styles.danger}
          title={t("seed.clearAllTitle")}
        >
          {resetting ? "…" : t("seed.clearAll")}
        </button>
      </div>

      <div style={styles.row}>
        <span style={styles.label}>{t("seed.barrier")}</span>
        <input
          type="range"
          min={10}
          max={90}
          value={barrierDeg}
          onChange={(e) => setBarrierDeg(Number(e.target.value))}
          style={{ flex: 1 }}
        />
        <span style={styles.val}>{barrierDeg}°</span>
      </div>
      <div style={styles.row}>
        <span style={styles.label} title={t("seed.barrierHint")}>
          {t("seed.barrierHint")}
        </span>
      </div>

      <div style={styles.row}>
        <label style={{ ...styles.label, display: "flex", alignItems: "center", gap: 6 }}>
          <input
            type="checkbox"
            checked={optimizer}
            onChange={(e) => setOptimizer(e.target.checked)}
          />
          {t("seed.optimizer")}
        </label>
      </div>

      <div style={styles.buttonRow}>
        <button onClick={onGrow} disabled={growing} style={styles.grow}>
          {growing ? "…" : t("seed.grow")}
        </button>
        <button onClick={clearSeedPoints} disabled={seedPoints.length === 0} style={styles.clear}>
          {t("seed.clear")}
        </button>
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  panel: {
    position: "absolute",
    bottom: 16,
    left: "50%",
    transform: "translateX(-50%)",
    background: "var(--bg-panel, #2d2d2d)",
    border: "1px solid var(--border, #555)",
    borderRadius: 10,
    padding: "10px 14px",
    color: "var(--text-1, #eee)",
    fontSize: 12,
    width: 360,
    maxWidth: "92vw",
    boxShadow: "0 4px 20px rgba(0,0,0,0.4)",
    zIndex: 20,
  },
  title: { fontWeight: 700, fontSize: 13, marginBottom: 4 },
  titleRow: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    marginBottom: 4,
  },
  closeBtn: {
    width: 22,
    height: 22,
    border: "1px solid var(--border, #555)",
    borderRadius: 11,
    background: "transparent",
    color: "var(--text-2, #ccc)",
    cursor: "pointer",
    fontSize: 14,
    lineHeight: "20px",
    padding: 0,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
  },
  hint: { color: "var(--text-3, #aaa)", fontSize: 11, lineHeight: 1.5, marginBottom: 8 },
  row: { display: "flex", alignItems: "center", gap: 8, marginBottom: 6 },
  label: { flex: "0 0 auto", whiteSpace: "nowrap" },
  val: { flex: "0 0 auto", width: 32, textAlign: "right", color: "var(--text-2, #ccc)" },
  buttonRow: { display: "flex", gap: 8, marginTop: 6 },
  fuse: {
    flex: 1,
    padding: "8px 10px",
    borderRadius: 6,
    border: "1px solid #a78bfa",
    background: "linear-gradient(90deg, #7c3aed, #2563eb)",
    color: "#fff",
    cursor: "pointer",
    fontSize: 13,
    fontWeight: 700,
  },
  danger: {
    flex: 1,
    padding: "6px 10px",
    borderRadius: 6,
    border: "1px solid #ff4d4f",
    background: "transparent",
    color: "#ff4d4f",
    cursor: "pointer",
    fontSize: 12,
    fontWeight: 600,
  },
  grow: {
    flex: 1,
    padding: "6px 10px",
    borderRadius: 6,
    border: "1px solid var(--accent, #4a9eff)",
    background: "var(--accent, #4a9eff)",
    color: "#fff",
    cursor: "pointer",
    fontSize: 13,
  },
  clear: {
    padding: "6px 10px",
    borderRadius: 6,
    border: "1px solid var(--border, #555)",
    background: "transparent",
    color: "var(--text-2, #ccc)",
    cursor: "pointer",
    fontSize: 13,
  },
  toggle: {
    padding: "6px 8px",
    borderRadius: 6,
    border: "1px solid var(--border, #555)",
    cursor: "pointer",
    fontSize: 12,
    minWidth: 56,
    textAlign: "center",
    transition: "background 0.15s, color 0.15s",
  },
  miniBtn: {
    padding: "4px 8px",
    borderRadius: 4,
    border: "1px solid var(--border, #555)",
    background: "transparent",
    color: "var(--text-2, #ccc)",
    cursor: "pointer",
    fontSize: 12,
    flex: 1,
  },
};
