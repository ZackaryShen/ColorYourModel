import { useEffect, useRef, useState } from "react";
import { useAppStore } from "../store/appStore";
import { PaintTool, SeedPoint } from "../types/mesh";
import { useTauriCommand } from "../hooks/useTauriCommand";
import { useT } from "../i18n";
import { log } from "../utils/logger";

const POS_KEY = "cym.seedPanelPos";
const PANEL_WIDTH = 360;
const PANEL_HEIGHT_ESTIMATE = 580; // fallback when the DOM has not measured yet
const PANEL_MARGIN = 16;

/// At-rest anchor (iteration 79): bottom-right with a sensible inset. The
/// bottom-of-window placement was wrong on tall / multi-monitor windows — the
/// panel always ended up in the lower-mid area, which on a 4K display is well
/// away from where the user is currently looking (which is the model, dead
/// centre). Bottom-right anchors near the action area but never hides the
/// canvas centre.
const DEFAULT_POS = (): { x: number; y: number } => {
  const w = window.innerWidth;
  const h = window.innerHeight;
  // Prefer the right edge, leaving a small inset so the close button is not
  // under the OS scrollbar/chrome. If the window is so narrow the panel
  // wouldn't fit, fall back to the centred x.
  const x = Math.max(
    PANEL_MARGIN,
    Math.round(w - PANEL_WIDTH - PANEL_MARGIN),
  );
  const y = Math.max(PANEL_MARGIN, Math.round(h - PANEL_HEIGHT_ESTIMATE - PANEL_MARGIN));
  return { x, y };
};

function loadPos(panelEl?: HTMLElement | null): { x: number; y: number } {
  try {
    const raw = localStorage.getItem(POS_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as { x?: number; y?: number };
      if (typeof parsed.x === "number" && typeof parsed.y === "number") {
        return clampPos(parsed.x, parsed.y, panelEl);
      }
    }
  } catch {
    /* corrupt entry — fall through to default */
  }
  return DEFAULT_POS();
}

function clampPos(
  x: number,
  y: number,
  panelEl?: HTMLElement | null,
): { x: number; y: number } {
  // Measure the live panel when available so a saved corner never escapes a
  // later-shrunken window AND a tall panel never overflows the bottom. When
  // the DOM hasn't measured yet (first paint), fall back to the static
  // PANEL_WIDTH/PANEL_HEIGHT_ESTIMATE so the layout doesn't jump.
  const rect = panelEl?.getBoundingClientRect();
  const w = rect?.width ?? PANEL_WIDTH;
  const h = rect?.height ?? PANEL_HEIGHT_ESTIMATE;
  const maxX = Math.max(PANEL_MARGIN, window.innerWidth - w - PANEL_MARGIN);
  const maxY = Math.max(PANEL_MARGIN, window.innerHeight - h - PANEL_MARGIN);
  return {
    x: Math.min(Math.max(PANEL_MARGIN, x), maxX),
    y: Math.min(Math.max(PANEL_MARGIN, y), maxY),
  };
}

/// Seeded-watershed control panel (iteration 50). Shown while the Seed tool is
/// active. The user places seed points on the mesh (handled in Viewport), and
/// this panel tunes the barrier angle, toggles the optimizer, and triggers grow.
///
/// Drift history: iteration 78 — panel is now a movable / draggable overlay
/// (title bar = drag handle; double-click title = reset position) so users can
/// move it out of the way of the 3D viewport they are trying to inspect.
///
/// Drift history: iteration 78 — adds a "Pick a partition" sub-mode
/// (mutually exclusive with the existing erase sub-mode). While ON, clicking a
/// face on the mesh sets `selectedSegment` to that face's label and
/// auto-exits pick mode — the direct in-context path for "select this region
/// and run eye detect", replacing the old "open SegmentsPanel and click a
/// row" detour.
export function SeedPanel() {
  const t = useT();
  const seedPoints = useAppStore((s) => s.seedPoints);
  const clearSeedPoints = useAppStore((s) => s.clearSeedPoints);
  const seedEraseMode = useAppStore((s) => s.seedEraseMode);
  const setSeedEraseMode = useAppStore((s) => s.setSeedEraseMode);
  const seedPickMode = useAppStore((s) => s.seedPickMode);
  const setSeedPickMode = useAppStore((s) => s.setSeedPickMode);
  const segmentMode = useAppStore((s) => s.segmentMode);
  const setSegmentMode = useAppStore((s) => s.setSegmentMode);
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
  const { seedGrow, recommendSeeds, detectPlanarRegions, detectMultiViewRegions, detectCrossSectionRegions, detectEyeRegions, detectEyeRegionsAuto, fuseSegmentation, resetSegmentation } = useTauriCommand();

  // Iteration 66: give the panel an obvious "I'm done here" exit affordance.
  // The × button on the title row and the Esc key both switch back to View.
  // Seed / ghost / algorithm region state is left in the store so re-entering
  // the Seed tool restores the in-progress workflow.
  const closePanel = () => {
    log.info("SeedPanel", "closePanel → View");
    setSeedPickMode(false); // disarm before unmount so re-entering Seed tool doesn't start in Pick
    setActiveTool(PaintTool.View);
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        log.info("SeedPanel", "Esc pressed → View");
        setSeedPickMode(false); // same disarm reason as closePanel
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
  const [dihedralDeg, setDihedralDeg] = useState(15);
  const [resetting, setResetting] = useState(false);

  // ── Drag-to-move (iteration 78) ────────────────────────────────────────
  // Position is a React state so styles/persistence see the final value, but
  // we drive the actual motion via direct DOM writes + a window listener so
  // the panel tracks the cursor at full frame rate without React re-renders.
  // Persistence happens once on pointerup, not on every move.
  //
  // Iteration 79: live drag clamp uses the panel's *measured* rect width/height
  // (not `window.innerWidth-200 / window.innerHeight-80` like before), so a
  // tall panel can never drift past the bottom of a short window and a wide
  // one past the right.
  const panelRef = useRef<HTMLDivElement | null>(null);
  const [pos, setPos] = useState<{ x: number; y: number }>(() =>
    loadPos(null),
  );

  // Iteration 80: stash the most recent `fuse-debug` payload (emitted by
  // `fuse_segmentation`) on a module-scoped slot so the post-fuse status
  // message can read it. We avoid putting it in component state to keep the
  // listener registration side-effect free.
  useEffect(() => {
    type FuseDebugPayload = {
      channels?: {
        planar?: number;
        multiview?: number;
        dihedral?: number;
        eye?: number;
      };
      rawComponents?: number;
      cutThreshold?: number;
      minRegionFaces?: number;
      edgeTotal?: number;
      edgeCut?: number;
      planarVoteEdges?: number;
      planarCutVotes?: number;
      multiviewVoteEdges?: number;
      multiviewCutVotes?: number;
      dihedralCutVotes?: number;
      eyeCutVotes?: number;
      regionSizeMin?: number;
      regionSizeMax?: number;
      regionSizeMedian?: number;
      regionsBeforeMerge?: number;
      regionsAfterMerge?: number;
      mergePasses?: number;
      minFaces?: number;
      tinyRegionsBeforeMerge?: number;
    };
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        if (cancelled) return;
        const h = await listen<FuseDebugPayload>("fuse-debug", (e) => {
          const p = e.payload;
          // Iteration 82: ship the structured breakdown into the in-app
          // debug-log ring buffer (renders in the purple DebugLogViewer
          // panel) AND the browser console. The transient status bar line
          // scrolls away on the next action, so a durable, copyable log
          // entry is the only way to read *why* the button produced e.g.
          // 535 regions on a smooth model.
          const c = p.channels ?? {};
          const msg =
            `[fuse-debug] regions=${p.rawComponents} ` +
            `channels[planar=${c.planar} multiview=${c.multiview} dihedral=${c.dihedral} eye=${c.eye}] ` +
            `edges(cut/total)=${p.edgeCut}/${p.edgeTotal} ` +
            `votes[planar cut=${p.planarCutVotes}/${p.planarVoteEdges} ` +
            `multiview cut=${p.multiviewCutVotes}/${p.multiviewVoteEdges} ` +
            `dihedral=${p.dihedralCutVotes} eye=${p.eyeCutVotes}] ` +
            `regionSizes[min=${p.regionSizeMin} med=${p.regionSizeMedian} max=${p.regionSizeMax}] ` +
            `merge[before=${p.regionsBeforeMerge} after=${p.regionsAfterMerge} passes=${p.mergePasses} tiny=${p.tinyRegionsBeforeMerge} minFaces=${p.minFaces}] ` +
            `cutThreshold=${p.cutThreshold} minRegionFaces=${p.minRegionFaces}`;
          log.info("fuse-debug", msg, p);
          // Latest-write-wins; the post-fuse status is read after `await fuse...`
          // resolves.
          (window as unknown as { __lastFuseDebug?: FuseDebugPayload }).__lastFuseDebug =
            p;
        });
        unlisten = h;
      } catch {
        /* non-Tauri context (unit tests): ignore */
      }
    })();
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);
  const dragRef = useRef<{
    pointerId: number;
    startClientX: number;
    startClientY: number;
    startPanelX: number;
    startPanelY: number;
  } | null>(null);
  const [dragging, setDragging] = useState(false);

  // Live dimensions of the panel, refreshed on every render once it has
  // mounted. Falls back to the static PANEL_* constants until then.
  const liveRect = panelRef.current?.getBoundingClientRect();
  const liveW = liveRect?.width ?? PANEL_WIDTH;
  const liveH = liveRect?.height ?? PANEL_HEIGHT_ESTIMATE;
  const clampLive = (x: number, y: number) => {
    const maxX = Math.max(
      PANEL_MARGIN,
      window.innerWidth - liveW - PANEL_MARGIN,
    );
    const maxY = Math.max(
      PANEL_MARGIN,
      window.innerHeight - liveH - PANEL_MARGIN,
    );
    return {
      x: Math.min(Math.max(PANEL_MARGIN, x), maxX),
      y: Math.min(Math.max(PANEL_MARGIN, y), maxY),
    };
  };

  // First-mount re-clamp (iteration 79). Position was computed from the
  // static PANEL_* fallback constants; once the panel mounts and React commits
  // the real DOM, re-clamp against the measured rect so the saved corner
  // does not leak past the actual panel right/bottom.
  useEffect(() => {
    if (!panelRef.current) return;
    const r = panelRef.current.getBoundingClientRect();
    if (r.width === liveW && r.height === liveH && liveRect) {
      // Already using live dimensions; nothing to do.
      return;
    }
    setPos((prev) => {
      const next = clampLive(prev.x, prev.y);
      if (next.x === prev.x && next.y === prev.y) return prev;
      try {
        localStorage.setItem(POS_KEY, JSON.stringify(next));
      } catch {
        /* ignore */
      }
      return next;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [liveW, liveH]);

  useEffect(() => {
    const onMove = (e: PointerEvent) => {
      const ds = dragRef.current;
      if (!ds || e.pointerId !== ds.pointerId) return;
      const dx = e.clientX - ds.startClientX;
      const dy = e.clientY - ds.startClientY;
      const nx = Math.min(
        Math.max(PANEL_MARGIN, ds.startPanelX + dx),
        Math.max(PANEL_MARGIN, window.innerWidth - liveW - PANEL_MARGIN),
      );
      const ny = Math.min(
        Math.max(PANEL_MARGIN, ds.startPanelY + dy),
        Math.max(PANEL_MARGIN, window.innerHeight - liveH - PANEL_MARGIN),
      );
      const el = panelRef.current;
      if (el) {
        el.style.left = `${nx}px`;
        el.style.top = `${ny}px`;
        el.style.transform = "none";
      }
    };
    const onUp = (e: PointerEvent) => {
      const ds = dragRef.current;
      if (!ds || e.pointerId !== ds.pointerId) return;
      const el = panelRef.current;
      if (el) {
        const rect = el.getBoundingClientRect();
        const final = clampLive(rect.left, rect.top);
        setPos(final);
        try {
          localStorage.setItem(POS_KEY, JSON.stringify(final));
        } catch {
          /* private mode etc — not critical */
        }
      }
      dragRef.current = null;
      setDragging(false);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onUp);
    };
  }, [liveW, liveH]);

  // Re-clamp on viewport resize so a saved position never escapes the window
  // when the user shrinks the browser after re-opening the app. Iteration 79:
  // rAF-throttled so a fast resize doesn't fire 30 setState calls per second,
  // and uses the live panel rect (so a recently-grown or shrunk panel is
  // measured, not assumed to be the static default).
  useEffect(() => {
    let raf = 0;
    const onResize = () => {
      if (raf) return;
      raf = window.requestAnimationFrame(() => {
        raf = 0;
        setPos((prev) => {
          const next = clampLive(prev.x, prev.y);
          if (next.x === prev.x && next.y === prev.y) return prev;
          try {
            localStorage.setItem(POS_KEY, JSON.stringify(next));
          } catch {
            /* ignore */
          }
          return next;
        });
      });
    };
    window.addEventListener("resize", onResize);
    return () => {
      if (raf) window.cancelAnimationFrame(raf);
      window.removeEventListener("resize", onResize);
    };
  }, [liveW, liveH]);

  const onTitlePointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return; // left button only
    // Don't capture if the user is interacting with the close/reset buttons —
    // those are positioned inside the title row but use a child-stop propagation.
    if ((e.target as HTMLElement).closest("button")) return;
    const rect = panelRef.current?.getBoundingClientRect();
    if (!rect) return;
    dragRef.current = {
      pointerId: e.pointerId,
      startClientX: e.clientX,
      startClientY: e.clientY,
      startPanelX: rect.left,
      startPanelY: rect.top,
    };
    setDragging(true);
    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch {
      /* not critical */
    }
    e.preventDefault();
  };

  const resetPosition = () => {
    // Iteration 79: anchor the panel using its measured rect when available,
    // so reset always lands the panel *fully visible* in the current window.
    // Without this, a panel that's wider than the assumed PANEL_WIDTH (e.g.
    // after font-size scaling) would reset to a clipped or out-of-bounds
    // position on tall / multi-monitor setups.
    const def = clampLive(
      window.innerWidth - liveW - PANEL_MARGIN,
      window.innerHeight - liveH - PANEL_MARGIN,
    );
    setPos(def);
    try {
      localStorage.setItem(POS_KEY, JSON.stringify(def));
    } catch {
      /* ignore */
    }
    const el = panelRef.current;
    if (el) {
      el.style.left = `${def.x}px`;
      el.style.top = `${def.y}px`;
      el.style.transform = "none";
    }
    log.info("SeedPanel", "panel position reset", def);
  };

  // ── Sub-mode toggles (iteration 78) ────────────────────────────────────
  // Pick and Erase are mutually exclusive — turning one on auto-clears the
  // other. Without the auto-clear, both styles fight on the title row and the
  // user can't tell which sub-mode is active.
  const onToggleErase = () => {
    const next = !seedEraseMode;
    setSeedEraseMode(next);
    if (next) setSeedPickMode(false);
  };
  const onTogglePick = () => {
    const next = !seedPickMode;
    setSeedPickMode(next);
    if (next) setSeedEraseMode(false);
  };

  // Switching out of the Seed tool (Esc / ×) must not leave Pick mode armed —
  // otherwise re-entering Seed tool would silently start in pick mode and the
  // next mesh click would silently set selectedSegment without the user
  // asking for it. (Disarm also happens in closePanel / Esc handler above.)
  useEffect(() => {
    return () => setSeedPickMode(false);
  }, []);

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
  // faces belonging to the CURRENTLY SELECTED partition — set either by clicking
  // a row in SegmentsPanel or (iteration 78) by the new Pick-for-Eye sub-mode
  // of the Seed tool. We derive the ROI faces from `segmentLabels` (the per-face
  // partition id array) intersected with `selectedSegment`; no extra backend
  // round-trip needed. The eye detector is read-only, so its regions are stored
  // purely for the Viewport overlay (like Layer 1/2/3).
  const onDetectEye = async () => {
    setDetectingEye(true);
    log.info("SeedPanel", "onDetectEye click", { selectedSegment });
    // Loud-without-flicker: surface the pre-condition failure to BOTH the JS
    // console AND the status bar (the status bar message gets overwritten by
    // other commands, but the console log is permanent and lets a user paste
    // the exact reason without staring at the panel).
    const md = meshData;
    if (selectedSegment === null || !md || !md.segmentLabels) {
      const msg = `[eye] no selected partition (selectedSegment=${selectedSegment}, mesh=${md ? "ok" : "null"}, labels=${md?.segmentLabels ? "ok" : "null"}). Click \"Pick a partition\" then click the eye-bounding region on the model.`;
      log.error("SeedPanel", "onDetectEye blocked: no selected partition", {
        selectedSegment,
        hasMesh: !!md,
        hasLabels: !!(md && md.segmentLabels),
      });
      // eslint-disable-next-line no-console
      console.error(msg);
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

  // Stage 1: global eye detection without a user-supplied ROI. The result is
  // stored in the same `eyeRegions` slot and participates in fusion / growth.
  const onDetectEyeAuto = async () => {
    setDetectingEye(true);
    log.info("SeedPanel", "onDetectEyeAuto click");
    try {
      const regions = await detectEyeRegionsAuto();
      log.info("SeedPanel", "onDetectEyeAuto received", { regions: regions.length });
      // REFUTE: do NOT stack eye-region seeds on top of a large suggestion set
      // (e.g. 125 MultiView seeds). `seed_grow` is a global geodesic Voronoi;
      // 125 + 4 seeds = ~128 tiny regions covering the whole mesh, which is
      // exactly the "碎成彩虹" failure in the user report. Eye regions are
      // semantic constraints for fusion / per-region commitment, not generic
      // watershed seeds, so we clear the advisory suggestion pool first.
      clearSuggestedSeeds();
      clearPlanarRegions();
      clearMultiviewRegions();
      setEyeRegions(regions);
    } catch {
      // error already surfaced via status message in detectEyeRegionsAuto
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
    log.info("SeedPanel", "onFuse click", {
      eyeRegions: eyeRegions.length,
    });
    try {
      // Iteration 79: feed any user-confirmed eye regions into the fuse as a
      // 5th channel. Without this the eye-region boundary lines on the
      // viewport are *visualisation only* — on the next fuse or seedGrow run
      // the eye faces get merged into whatever neighbour wins the
      // geodesic-nearest race. With this, every perimeter edge of an eye
      // region votes cut (weight 1000, same magnitude as the dihedral
      // backbone), so the eye region stays its own manual label even when the
      // fuse pass would otherwise absorb it.
      const eyeSets = eyeRegions.map((r) => r.faceIndices);
      // Iteration 84: cut-threshold default raised 1 → 2 (true majority on the
      // weak channels). On this model the dominant cutters are the weighted
      // dihedral/eye channels (WEIGHT = 1000), which ignore this threshold, so
      // the visible de-fragmentation comes from the rewritten tiny-region merge
      // in fuse.rs — not from this number. Kept at 2 per the "C" plan so a lone
      // planar/multiview vote (weight 1) still never cuts on its own.
      const result = await fuseSegmentation(2, 0, dihedralDeg, eyeSets);
      log.info("SeedPanel", "onFuse done", {
        segments: result.segments.length,
        eyeChannels: eyeSets.length,
      });
      // Iteration 80: help the user debug the common "fuse produced 500 regions
      // on a smooth model" failure by surfacing the channel + cut breakdown
      // the backend emitted on `fuse-debug`. Most of the time the answer is
      // "raise min_region_faces" (the sliver slider below) rather than
      // tweaking the algorithms themselves.
      const fuseLast = (window as unknown as { __lastFuseDebug?: {
        channels?: { planar?: number; multiview?: number; dihedral?: number; eye?: number };
        edgeTotal?: number;
        edgeCut?: number;
        regionSizeMin?: number;
        regionSizeMax?: number;
        regionSizeMedian?: number;
      } }).__lastFuseDebug;
      if (fuseLast && typeof fuseLast === "object") {
        const c = fuseLast.channels || {};
        const ec = fuseLast.edgeTotal ?? 0;
        const cut = fuseLast.edgeCut ?? 0;
        const min = fuseLast.regionSizeMin ?? 0;
        const med = fuseLast.regionSizeMedian ?? 0;
        const mx = fuseLast.regionSizeMax ?? 0;
        setStatusMessage(
          `🧩 融合完成：${result.segments.length} 区（通道 平面${c.planar ?? 0}/多视角${c.multiview ?? 0}/折痕${c.dihedral ?? 0}/眼${c.eye ?? 0}, 边 ${cut}/${ec} 切, 区大小 min=${min} med=${med} max=${mx}）`,
        );
      }
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
      setSeedPickMode(false);
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
      eyeRegions: eyeRegions.length,
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
    //
    // Iteration 79: also include one auto-seed per detected EyeRegion (anchor
    // at the median face of the region). Without this, seedGrow's geodesic
    // nearest-seed Voronoi would happily reassign every eye face to the
    // nearest body / face seed — that's exactly the "eye regions vanish after
    // grow" failure mode the user reported. An eye-region seed reserves its
    // label the same way a manual click would, so the eye boundary lines on
    // the viewport survive the grow without surprising the user with extra
    // ghost markers they have to click-and-accept.
    const eyeSeeds: SeedPoint[] = eyeRegions
      .map((r): SeedPoint | null => {
        if (!r.faceIndices.length) return null;
        // Median face inside the region. Not strictly a centroid but always
        // interior for a connected EyeRegion (the only kind the detector
        // emits), which is enough to anchor Dijkstra without falling on a
        // boundary edge.
        const f = r.faceIndices[Math.floor(r.faceIndices.length / 2)];
        const md = meshData;
        if (!md) return null;
        // meshData.faces / vertices are flat float arrays: each face is 3
        // consecutive vertex indices, each vertex is xyz. Index into the flat
        // arrays the same way `MeshModel::face_centroid` does on the backend.
        const i = f * 3;
        const v0 = md.faces[i];
        const v1 = md.faces[i + 1];
        const v2 = md.faces[i + 2];
        const j0 = v0 * 3;
        const j1 = v1 * 3;
        const j2 = v2 * 3;
        const vs = md.vertices;
        if (
          j0 + 2 >= vs.length ||
          j1 + 2 >= vs.length ||
          j2 + 2 >= vs.length
        ) {
          return null;
        }
        return {
          x: (vs[j0] + vs[j1] + vs[j2]) / 3,
          y: (vs[j0 + 1] + vs[j1 + 1] + vs[j2 + 1]) / 3,
          z: (vs[j0 + 2] + vs[j1 + 2] + vs[j2 + 2]) / 3,
          faceIndex: f,
        };
      })
      .filter((s): s is SeedPoint => s !== null);

    // Eye seeds are semantic "reservations", not generic watershed seeds. If
    // they are the ONLY seeds, seed_grow would assign every face in the mesh to
    // the nearest eye centre (wrong: body/limbs become eye labels). Only let
    // them join the pool when there are real body seeds (manual or suggested)
    // to grow from; otherwise nudge the user toward the correct tool.
    const hasBodySeeds = seedPoints.length > 0 || suggestedSeeds.length > 0;
    if (eyeSeeds.length > 0 && !hasBodySeeds) {
      setStatusMessage(t("seed.eyeNeedBodySeeds"));
      return;
    }

    const seeds = [...seedPoints, ...suggestedSeeds, ...eyeSeeds];
    if (seeds.length === 0) {
      setStatusMessage(t("seed.needOne"));
      return;
    }
    setGrowing(true);
    setStatusMessage(
      `🌱 准备生长（${seeds.length} 个种子（手 ${seedPoints.length} + 推 ${suggestedSeeds.length} + 眼 ${eyeSeeds.length}），barrier=${barrierDeg}°，optimizer=${optimizer}）…`,
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

  // The body of the hint switches based on whichever view / overlay / sub-mode
  // is currently relevant. Iteration 78: pick mode now owns its own line in
  // the priority list — the user has to see that "clicking now picks" BEFORE
  // they would otherwise read "clicking now places a seed".
  const hint = (() => {
    if (seedPickMode) return t("seed.eyePickHint");
    if (seedEraseMode) return t("seed.eraseHint");
    if (eyeRegions.length > 0) return t("seed.eyeHint");
    if (crossSectionRegions.length > 0) return t("seed.crossSectionHint");
    if (multiviewRegions.length > 0) return t("seed.multiviewHint");
    if (planarRegions.length > 0) return t("seed.planarHint");
    if (suggestedSeeds.length > 0) return t("seed.suggestHint");
    return t("seed.hint");
  })();

  return (
    <div
      ref={panelRef}
      style={{
        ...styles.panel,
        left: pos.x,
        top: pos.y,
        transform: "none",
        boxShadow: dragging
          ? "0 12px 36px rgba(0,0,0,0.55)"
          : "0 4px 20px rgba(0,0,0,0.4)",
      }}
    >
      <div
        onPointerDown={onTitlePointerDown}
        onDoubleClick={resetPosition}
        style={{
          ...styles.titleRow,
          cursor: dragging ? "grabbing" : "grab",
          userSelect: "none",
        }}
        title={t("seed.pickPanelTitle")}
      >
        <span style={styles.title}>🌱 {t("tool.seed")}</span>
        <button
          onClick={resetPosition}
          style={styles.resetBtn}
          title={t("seed.panelResetTitle")}
          aria-label={t("seed.panelResetTitle")}
        >
          {t("seed.panelReset")}
        </button>
        <button
          onClick={closePanel}
          style={styles.closeBtn}
          title={t("seed.closeTitle")}
          aria-label={t("seed.closeTitle")}
        >
          ×
        </button>
      </div>
      <div style={styles.hint}>{hint}</div>

      {/* Iteration B: explicit two-technique switcher. `auto` = fuse (global
          edge-vote, one click, no seeds); `manual` = grow (geodesic watershed
          from seed points / suggested seeds). They SHARE the eye-region
          reservation but are otherwise disjoint: planar / multiview /
          cross-section detectors feed the manual seed pool, the dihedral slider
          feeds fuse only. */}
      <div style={styles.modeTabs}>
        <button
          onClick={() => setSegmentMode("auto")}
          style={{ ...styles.modeTab, ...(segmentMode === "auto" ? styles.modeTabActive : {}) }}
          title={t("seed.modeAutoTitle")}
        >
          🧩 {t("seed.modeAuto")}
        </button>
        <button
          onClick={() => setSegmentMode("manual")}
          style={{ ...styles.modeTab, ...(segmentMode === "manual" ? styles.modeTabActive : {}) }}
          title={t("seed.modeManualTitle")}
        >
          🌱 {t("seed.modeManual")}
        </button>
      </div>
      <div style={styles.modeHint}>
        {segmentMode === "auto" ? t("seed.modeAutoHint") : t("seed.modeManualHint")}
      </div>

      {segmentMode === "manual" && (
        <>
        <div style={styles.row}>
        <span style={styles.label}>{t("seed.count", seedPoints.length)}</span>
        <button
          onClick={onToggleErase}
          style={{
            ...styles.subMode,
            ...(seedEraseMode ? styles.subModeActiveErase : {}),
          }}
          title={t("seed.eraseHint")}
        >
          🩹 {t("seed.eraseMode")}
        </button>
        <button
          onClick={onTogglePick}
          style={{
            ...styles.subMode,
            ...(seedPickMode ? styles.subModeActivePick : {}),
          }}
          title={seedPickMode ? t("seed.pickModeActive") : t("seed.pickMode")}
        >
          {seedPickMode ? "✓" : "🖱"} {t("seed.pickMode")}
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
        </>
      )}

      <div style={styles.row}>
        <button
          onClick={onDetectEyeAuto}
          disabled={detectingEye}
          style={{
            ...styles.grow,
            background: "#2a1a4a",
          }}
          title={t("seed.eyeAutoTitle")}
        >
          {detectingEye ? "…" : t("seed.eyeAuto")}
        </button>
      </div>

      <div style={styles.row}>
        <button
          onClick={onDetectEye}
          disabled={detectingEye || selectedSegment === null}
          style={{
            ...styles.grow,
            // Visual cue that the button is unusable because nothing is selected
            opacity: selectedSegment === null ? 0.5 : 1,
          }}
          title={
            selectedSegment === null
              ? t("seed.eyeNeedSegmentShort")
              : t("seed.eyeTitle")
          }
        >
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

      {/* Persistent red hint that explains why Eye detect is inert — the
          status-bar toast gets overwritten by other commands, but this is
          unmistakable. Iteration 78 also routes the user to the new Pick mode
          (when no segment is selected AND pick mode isn't already active),
          instead of forcing them to hunt for the SegmentsPanel row. */}
      {selectedSegment === null && eyeRegions.length === 0 && !seedPickMode && (
        <div
          style={{
            marginTop: 6,
            padding: "6px 10px",
            background: "#3b0a14",
            color: "#fecaca",
            border: "1px solid #f87171",
            borderRadius: 6,
            fontSize: 12,
            lineHeight: 1.4,
          }}
        >
          {t("seed.eyeNeedSegmentHint")}
        </div>
      )}

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

      {segmentMode === "auto" && (
        <>
          {/* Geometry backbone for the fusion: dihedral crease angle. on smooth /
              single-colour meshes the planar + multiview channels have no signal,
              so this is what actually splits the model into parts. Lower = more,
              finer regions; higher = fewer, coarser parts. */}
          <div style={styles.row}>
            <span style={styles.label} title={t("seed.fuseDihedralHint")}>
              {t("seed.fuseDihedral")}
            </span>
            <input
              type="range"
              min={5}
              max={35}
              value={dihedralDeg}
              onChange={(e) => setDihedralDeg(Number(e.target.value))}
              style={{ flex: 1 }}
            />
            <span style={styles.val}>{dihedralDeg}°</span>
          </div>

          {/* Layer 4 fusion — one click turns the algorithm regions into the
              actual partition, no seed placement required. */}
          <div style={styles.row}>
            <button onClick={onFuse} disabled={fusing} style={styles.fuse} title={t("seed.fuseTitle")}>
              {fusing ? "…" : t("seed.fuse")}
            </button>
          </div>
        </>
      )}

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

      {segmentMode === "manual" && (
        <>
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
        </>
      )}
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  panel: {
    position: "fixed",
    background: "var(--bg-panel, #2d2d2d)",
    border: "1px solid var(--border, #555)",
    borderRadius: 10,
    padding: "10px 14px",
    color: "var(--text-1, #eee)",
    fontSize: 12,
    width: 360,
    maxWidth: "92vw",
    boxShadow: "0 4px 20px rgba(0,0,0,0.4)",
    zIndex: 30,
  },
  title: { fontWeight: 700, fontSize: 13, marginBottom: 4 },
  modeTabs: {
    display: "flex",
    gap: 6,
    marginBottom: 4,
  },
  modeTab: {
    flex: 1,
    padding: "6px 8px",
    border: "1px solid var(--border, #555)",
    borderRadius: 6,
    background: "#2a2a2a",
    color: "#aaa",
    cursor: "pointer",
    fontSize: 12,
    fontWeight: 600,
  },
  modeTabActive: {
    borderColor: "var(--accent, #4a9eff)",
    background: "#3a5a7a",
    color: "#fff",
    boxShadow: "inset 0 0 0 1px var(--accent, #4a9eff)",
  },
  modeHint: {
    fontSize: 11,
    lineHeight: 1.35,
    color: "#9aa",
    background: "#22262b",
    border: "1px solid #3a3f47",
    borderRadius: 6,
    padding: "5px 8px",
    marginBottom: 8,
  },
  titleRow: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    marginBottom: 4,
    gap: 6,
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
    flexShrink: 0,
  },
  resetBtn: {
    width: 22,
    height: 22,
    border: "1px solid var(--border, #555)",
    borderRadius: 11,
    background: "transparent",
    color: "var(--text-2, #ccc)",
    cursor: "pointer",
    fontSize: 10,
    lineHeight: "20px",
    padding: 0,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    flexShrink: 0,
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
  // Sub-mode buttons (Place / Erase / Pick). When inactive they look like the
  // other secondary buttons; when active the colour signals which sub-mode is
  // armed so a single glance tells the user what the next mesh click will do.
  subMode: {
    flex: 1,
    padding: "5px 8px",
    borderRadius: 6,
    border: "1px solid var(--border, #555)",
    background: "transparent",
    color: "var(--text-2, #ccc)",
    cursor: "pointer",
    fontSize: 11,
    fontWeight: 500,
  },
  subModeActiveErase: {
    borderColor: "#ff4d4f",
    color: "#ff4d4f",
    background: "rgba(255, 77, 79, 0.08)",
  },
  subModeActivePick: {
    borderColor: "#c084fc",
    color: "#c084fc",
    background: "rgba(192, 132, 252, 0.10)",
  },
};