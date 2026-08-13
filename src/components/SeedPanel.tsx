import { useState } from "react";
import { useAppStore } from "../store/appStore";
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
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const { seedGrow, recommendSeeds } = useTauriCommand();

  const [barrierDeg, setBarrierDeg] = useState(45);
  const [optimizer, setOptimizer] = useState(false);
  const [growing, setGrowing] = useState(false);
  const [recommending, setRecommending] = useState(false);
  const [suggestCount, setSuggestCount] = useState(12);
  const [curvWeight, setCurvWeight] = useState(1.0);
  const [concWeight, setConcWeight] = useState(1.0);

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
      <div style={styles.title}>🌱 {t("tool.seed")}</div>
      <div style={styles.hint}>
        {seedEraseMode
          ? t("seed.eraseHint")
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
  hint: { color: "var(--text-3, #aaa)", fontSize: 11, lineHeight: 1.5, marginBottom: 8 },
  row: { display: "flex", alignItems: "center", gap: 8, marginBottom: 6 },
  label: { flex: "0 0 auto", whiteSpace: "nowrap" },
  val: { flex: "0 0 auto", width: 32, textAlign: "right", color: "var(--text-2, #ccc)" },
  buttonRow: { display: "flex", gap: 8, marginTop: 6 },
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
};
