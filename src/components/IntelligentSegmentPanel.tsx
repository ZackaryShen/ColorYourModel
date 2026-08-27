import { useEffect, useState } from "react";
import { useT } from "../i18n";
import { useAppStore } from "../store/appStore";
import { useTauriCommand } from "../hooks/useTauriCommand";
import {
  ALGORITHM_KINDS,
  DEFAULT_ALGORITHM_PARAMS,
  buildAlgorithm,
  type AlgorithmKind,
  type AlgorithmParams,
} from "../types/segment";
import { log } from "../utils/logger";

/**
 * Modal "pick an algorithm + tune its parameters" dialog that replaces the old
 * two-button (dihedral / SDF) segmentation entry points. Mirrors the
 * ExportDialog overlay/dialog pattern. The chosen `kind` + per-kind `params`
 * are persisted to the store so a model re-import auto-segments with the user's
 * last choice (no more hard-coded 30° dihedral) and the panel re-opens where
 * they left it.
 */
export function IntelligentSegmentPanel({ onClose }: { onClose: () => void }) {
  const t = useT();
  const { autoSegmentV2 } = useTauriCommand();
  const isLoaded = useAppStore((s) => s.isLoaded);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const setLoading = useAppStore((s) => s.setLoading);
  const lastAlgorithmParams = useAppStore((s) => s.lastAlgorithmParams);
  const lastSegmentKind = useAppStore((s) => s.lastSegmentKind);
  const setLastAlgorithmParams = useAppStore((s) => s.setLastAlgorithmParams);
  const setLastSegmentKind = useAppStore((s) => s.setLastSegmentKind);

  const [kind, setKind] = useState<AlgorithmKind>(lastSegmentKind ?? "curvatureKMeans");
  // Per-kind param record: switching algorithms and back keeps the sliders the
  // user already tuned (see AlgorithmParams in types/segment.ts).
  const [params, setParams] = useState<AlgorithmParams>(
    lastAlgorithmParams ?? DEFAULT_ALGORITHM_PARAMS
  );
  const [running, setRunning] = useState(false);

  // Close on Escape (real modal affordance; ExportDialog relies on overlay click).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Merge a partial patch into the *currently selected* kind's sub-object.
  // `params` always carries all kinds, so narrow-and-write is unnecessary.
  const patchKind = (patch: Record<string, number | boolean>) => {
    setParams((prev) => ({ ...prev, [kind]: { ...prev[kind], ...patch } }));
  };

  const run = async () => {
    if (!isLoaded || running) return;
    setRunning(true);
    // Flip the shared loading flag so the existing Viewport progress overlay
    // (driven by the `segment-progress` events the backend emits) actually
    // shows during the now-off-main-thread segmentation. Without this the panel
    // only had a binary "运行中" label and the overlay stayed hidden (REFUTE
    // major-3).
    setLoading(true);
    try {
      // Persist *before* running so a crash / cancel still remembers the choice.
      setLastAlgorithmParams(params);
      setLastSegmentKind(kind);
      const algorithm = buildAlgorithm(kind, params);
      await autoSegmentV2(algorithm);
      onClose();
    } catch (e) {
      log.error("IntelligentSegmentPanel", "run failed", { error: String(e) });
      setStatusMessage(`分区失败：${e}`);
    } finally {
      setRunning(false);
      setLoading(false);
    }
  };

  const algoLabel = (k: AlgorithmKind) => t(`segmentPanel.algo.${k}`);

  return (
    <div style={styles.overlay} onClick={onClose}>
      <div
        style={styles.dialog}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
      >
        <div style={styles.header}>{t("segmentPanel.title")}</div>

        {/* Algorithm selector */}
        <label style={styles.label}>{t("segmentPanel.algorithm")}</label>
        <select
          style={styles.select}
          value={kind}
          onChange={(e) => setKind(e.target.value as AlgorithmKind)}
        >
          {ALGORITHM_KINDS.map((k) => (
            <option key={k} value={k}>
              {algoLabel(k)}
            </option>
          ))}
        </select>

        {/* Plain-language description of the currently selected algorithm */}
        <div style={styles.algoDesc}>{t(`segmentPanel.algoDesc.${kind}`)}</div>

        {/* Per-algorithm parameters */}
        {kind === "dihedral" && (
          <Slider
            label={t("segmentPanel.angle")}
            value={params.dihedral.angleThreshold}
            min={1}
            max={179}
            step={1}
            onChange={(v) => patchKind({ angleThreshold: v })}
            suffix="°"
          />
        )}

        {kind === "shapeDiameter" && (
          <Slider
            label={t("segmentPanel.clusters")}
            hint={t("segmentPanel.clustersAuto")}
            value={params.shapeDiameter.k}
            min={0}
            max={24}
            step={1}
            onChange={(v) => patchKind({ k: v })}
          />
        )}

        {kind === "sdfGraphCut" && (
          <Slider
            label={t("segmentPanel.clusters")}
            hint={t("segmentPanel.clustersAuto")}
            value={params.sdfGraphCut.k}
            min={0}
            max={24}
            step={1}
            onChange={(v) => patchKind({ k: v })}
          />
        )}

        {kind === "concavity" && (
          <Slider
            label={t("segmentPanel.clusters")}
            hint={t("segmentPanel.clustersAuto")}
            value={params.concavity.k}
            min={0}
            max={48}
            step={1}
            onChange={(v) => patchKind({ k: v })}
          />
        )}

        {kind === "convexDecomposition" && (
          <>
            <Slider
              label={t("segmentPanel.maxHulls")}
              hint={t("segmentPanel.maxHullsAuto")}
              value={params.convexDecomposition.maxHulls}
              min={0}
              max={64}
              step={1}
              onChange={(v) => patchKind({ maxHulls: v })}
            />
            <Slider
              label={t("segmentPanel.concavityTol")}
              hint={t("segmentPanel.concavityTolHint")}
              value={params.convexDecomposition.concavity}
              min={1}
              max={20}
              step={1}
              onChange={(v) => patchKind({ concavity: v })}
              suffix="%"
            />
          </>
        )}

        {kind === "curveSkeleton" && (
          <>
            <Slider
              label={t("segmentPanel.maxHulls")}
              hint={t("segmentPanel.maxHullsAuto")}
              value={params.curveSkeleton.maxHulls}
              min={0}
              max={64}
              step={1}
              onChange={(v) => patchKind({ maxHulls: v })}
            />
            <Slider
              label={t("segmentPanel.concavityTol")}
              hint={t("segmentPanel.concavityTolHint")}
              value={params.curveSkeleton.concavity}
              min={1}
              max={20}
              step={1}
              onChange={(v) => patchKind({ concavity: v })}
              suffix="%"
            />
          </>
        )}

        {kind === "curvatureKMeans" && (
          <>
            <Slider
              label={t("segmentPanel.clusters")}
              hint={t("segmentPanel.clustersAuto")}
              value={params.curvatureKMeans.k}
              min={0}
              max={24}
              step={1}
              onChange={(v) => patchKind({ k: v })}
            />
            <Slider
              label={t("segmentPanel.smoothing")}
              value={params.curvatureKMeans.smoothingIters}
              min={0}
              max={10}
              step={1}
              onChange={(v) => patchKind({ smoothingIters: v })}
            />
            <Slider
              label={t("segmentPanel.crease")}
              hint={t("segmentPanel.creaseHint")}
              value={params.curvatureKMeans.creaseThresholdDeg}
              min={1}
              max={90}
              step={1}
              onChange={(v) => patchKind({ creaseThresholdDeg: v })}
              suffix="°"
            />
            <label style={styles.checkRow}>
              <input
                type="checkbox"
                checked={params.curvatureKMeans.useSdf}
                onChange={(e) => patchKind({ useSdf: e.target.checked })}
              />
              <span>{t("segmentPanel.useSdf")}</span>
            </label>
            <div style={styles.hint}>{t("segmentPanel.useSdfHint")}</div>
          </>
        )}

        {kind === "fhGraph" && (
          <>
            <Slider
              label={t("segmentPanel.fhScale")}
              hint={t("segmentPanel.fhScaleHint")}
              value={params.fhGraph.scale}
              min={0.05}
              max={1}
              step={0.05}
              onChange={(v) => patchKind({ scale: v })}
            />
            <Slider
              label={t("seed.weightCurv")}
              hint={t("seed.weightCurvHint")}
              value={params.fhGraph.curvature}
              min={0}
              max={2}
              step={0.1}
              onChange={(v) => patchKind({ curvature: v })}
            />
            <Slider
              label={t("seed.weightConc")}
              hint={t("seed.weightConcHint")}
              value={params.fhGraph.concavity}
              min={0}
              max={2}
              step={0.1}
              onChange={(v) => patchKind({ concavity: v })}
            />
          </>
        )}

        {!isLoaded && (
          <div style={styles.warn}>{t("segments.empty")}</div>
        )}

        <div style={styles.footer}>
          <button className="cym-btn" style={styles.btn} onClick={onClose}>
            {t("segmentPanel.cancel")}
          </button>
          <button
            className="cym-btn"
            style={{ ...styles.btn, ...styles.btnPrimary }}
            onClick={run}
            disabled={!isLoaded || running}
          >
            {running ? t("segmentPanel.running") : t("segmentPanel.run")}
          </button>
        </div>
      </div>
    </div>
  );
}

function Slider({
  label,
  hint,
  value,
  min,
  max,
  step,
  onChange,
  suffix,
}: {
  label: string;
  hint?: string;
  value: number;
  min: number;
  max: number;
  step: number;
  onChange: (v: number) => void;
  suffix?: string;
}) {
  return (
    <>
      <label style={styles.label}>
        {label}
        {hint && <span style={styles.hint}> — {hint}</span>}
      </label>
      <div style={styles.sliderRow}>
        <input
          type="range"
          style={styles.slider}
          min={min}
          max={max}
          step={step}
          value={value}
          onChange={(e) => onChange(Number(e.target.value))}
        />
        <span style={styles.value}>
          {value}
          {suffix ?? ""}
        </span>
      </div>
    </>
  );
}

const styles: Record<string, React.CSSProperties> = {
  overlay: {
    position: "fixed",
    inset: 0,
    background: "rgba(0,0,0,0.55)",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    zIndex: 1000,
  },
  dialog: {
    width: 420,
    maxHeight: "85vh",
    overflowY: "auto",
    background: "var(--bg-panel, #2d2d2d)",
    border: "1px solid var(--border, #555)",
    borderRadius: 10,
    padding: 16,
    color: "var(--text-1, #eee)",
    fontFamily: "inherit",
  },
  header: {
    fontSize: 15,
    fontWeight: 600,
    marginBottom: 12,
  },
  label: {
    display: "block",
    fontSize: 12,
    color: "var(--text-2, #aaa)",
    margin: "12px 0 4px",
  },
  hint: {
    color: "var(--text-3, #888)",
    fontSize: 11,
  },
  algoDesc: {
    marginTop: 8,
    padding: "8px 10px",
    background: "var(--bg-elevated, #3a3a3a)",
    borderLeft: "3px solid var(--accent, #4a9eff)",
    borderRadius: 6,
    fontSize: 12,
    lineHeight: 1.5,
    color: "var(--text-2, #aaa)",
  },
  select: {
    width: "100%",
    padding: "6px 8px",
    background: "var(--bg-elevated, #3a3a3a)",
    color: "var(--text-1, #eee)",
    border: "1px solid var(--border, #555)",
    borderRadius: 6,
    fontSize: 13,
  },
  sliderRow: {
    display: "flex",
    alignItems: "center",
    gap: 10,
  },
  slider: {
    flex: 1,
    accentColor: "var(--accent, #4a9eff)",
  },
  value: {
    minWidth: 42,
    textAlign: "right",
    fontSize: 13,
    color: "var(--text-1, #eee)",
    fontVariantNumeric: "tabular-nums",
  },
  checkRow: {
    display: "flex",
    alignItems: "center",
    gap: 8,
    marginTop: 12,
    fontSize: 13,
    color: "var(--text-1, #eee)",
    cursor: "pointer",
  },
  warn: {
    marginTop: 12,
    fontSize: 12,
    color: "var(--warn, #e0a030)",
  },
  footer: {
    display: "flex",
    justifyContent: "flex-end",
    gap: 8,
    marginTop: 18,
  },
  btn: {
    padding: "6px 14px",
    border: "none",
    borderRadius: 6,
    background: "var(--bg-hover, #444)",
    color: "var(--text-1, #eee)",
    cursor: "pointer",
    fontSize: 13,
  },
  btnPrimary: {
    background: "var(--accent, #4a9eff)",
    color: "#fff",
  },
};
