import { invoke } from "@tauri-apps/api/core";
import { useT, translateError } from "../i18n";
import { useAppStore } from "../store/appStore";
import { MeshData, ManualPointResult, Segment, SegmentResult, HistoryResult, HistoryState, SeedPoint, PlanarRegion, MultiViewRegion, CrossSectionRegion, EyeRegion } from "../types/mesh";
import type { ExportSelection } from "../types/export";
import type { SegmentationAlgorithm, SplitMethod, SplitResult } from "../types/segment";
import { log } from "../utils/logger";

export function useTauriCommand() {
  const t = useT();
  // Backend rejects carry stable English error strings (fuse/seeded/resegment);
  // localise them against the *live* language so a mid-flight switch still
  // renders the right one.
  const tErr = (e: unknown) => translateError(String(e), useAppStore.getState().language);
  const setMeshData = useAppStore((s) => s.setMeshData);
  const updateSegmentLabels = useAppStore((s) => s.updateSegmentLabels);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  // Drives the global ProgressBar overlay (Viewport). The lasso finalize
  // flips it so the backend's `manual:*` segment-progress events render as a
  // visible stage bar instead of a frozen click.
  const setLoading = useAppStore((s) => s.setLoading);
  const setSelectedSegment = useAppStore((s) => s.setSelectedSegment);
  const setSegmentMetadata = useAppStore((s) => s.setSegmentMetadata);
  const markHistoryDirty = useAppStore((s) => s.markHistoryDirty);
  const markPaintExported = useAppStore((s) => s.markPaintExported);
  const resetProjectTransientState = useAppStore((s) => s.resetProjectTransientState);
  const setToast = useAppStore((s) => s.setToast);

  const loadModel = async (path: string) => {
    log.info("useTauriCommand", `loadModel("${path}")`);
    try {
      setStatusMessage(t("toolbar.startImport"));
      const t0 = performance.now();
      const data = await invoke<MeshData>("load_model", { path });
      const dt = (performance.now() - t0).toFixed(1);

      log.info("useTauriCommand", `loadModel returned in ${dt}ms`, {
        vertices: data.vertices.length / 3,
        faces: data.faces.length / 3,
        faceColors: data.faceColors.length / 4,
        faceCount: data.faceCount,
        bbox: data.bbox,
      });

      setMeshData(data);
      log.info("useTauriCommand", "meshData stored in appStore");
      setStatusMessage(t("status.loaded", data.faceCount.toLocaleString()));
      return data;
    } catch (e) {
      log.error("useTauriCommand", "loadModel failed", { path, error: String(e) });
      setStatusMessage(`${t("toolbar.importFailed")}: ${e}`);
      throw e;
    }
  };

  /**
   * Wipe the current segmentation AND all face paint, returning the mesh to its
   * freshly-loaded "uncoloured" state (iteration 57, B2). Unlike the retired
   * autoSegmentV2 hook this DOES return the (neutral) face-colour buffer so the frontend repaints
   * back to grey in one shot — no model reload needed.
   */
  const resetSegmentation = async (): Promise<void> => {
    try {
      setStatusMessage(t("cmd.reset.working"));
      const result = await invoke<SegmentResult>("reset_segmentation");
      // faceColors length matches meshData.faceColors, so the store guard passes
      // and the canvas repaints to neutral grey.
      updateSegmentLabels(result.segmentLabels, result.segments, result.faceColors);
      setStatusMessage(t("cmd.reset.done"));
    } catch (e) {
      log.error("useTauriCommand", "resetSegmentation failed", { error: String(e) });
      setStatusMessage(t("cmd.reset.failed", String(e)));
      throw e;
    }
  };

  /**
   * Give a region a user-facing name. An empty string clears it back to the
   * generated "Region N".
   *
   * The backend owns the name table and hands back the whole segment list, so
   * the store is refreshed from the authoritative result rather than patched
   * optimistically — a rejected rename (region no longer exists after an undo)
   * must not leave the panel showing a name the backend never accepted.
   *
   * Returns whether it stuck, so the row can restore its previous text.
   */
  const renameSegment = async (segmentId: number, name: string): Promise<boolean> => {
    try {
      const segments = await invoke<Segment[]>("rename_segment", { segmentId, name });
      setSegmentMetadata(segments);
      return true;
    } catch (e) {
      log.error("useTauriCommand", "renameSegment failed", { segmentId, error: String(e) });
      setStatusMessage(t("cmd.rename.failed", String(e)));
      return false;
    }
  };

  /**
   * Absorb `sourceIds` into `targetId` so they become a single region.
   *
   * Labels move; paint does not. The response therefore carries no colour
   * buffer, and the store is refreshed through `updateSegmentLabels` with the
   * `faceColors` argument omitted — passing the old buffer back in would be
   * megabytes of IPC describing something that did not change.
   *
   * Returns how many faces moved, or null if the merge was rejected.
   */
  const mergeSegments = async (
    targetId: number,
    sourceIds: number[]
  ): Promise<number | null> => {
    try {
      const result = await invoke<{
        segments: Segment[];
        segmentLabels: number[];
        movedFaces: number;
      }>("merge_segments", { targetId, sourceIds });

      updateSegmentLabels(result.segmentLabels, result.segments);
      // The merge is on the backend timeline now, and it invalidated any redo
      // branch. Nothing else tells the toolbar that.
      markHistoryDirty();
      return result.movedFaces;
    } catch (e) {
      log.error("useTauriCommand", "mergeSegments failed", {
        targetId,
        sourceIds,
        error: String(e),
      });
      setStatusMessage(t("cmd.merge.failed", String(e)));
      return null;
    }
  };

  /**
   * Divide one region into sub-regions along its internal creases.
   *
   * Labels move; paint does not, so the response carries no colour buffer and the
   * store is refreshed through `updateSegmentLabels` with `faceColors` omitted.
   * Returns the result (with `movedFaces`/labels) or null if the split was
   * rejected. The split is on the backend undo timeline via `OpKind::Split`.
   */
  const splitSegment = async (
    segmentId: number,
    method: SplitMethod
  ): Promise<SplitResult | null> => {
    try {
      const result = await invoke<SplitResult>("split_segment", {
        label: segmentId,
        method,
      });

      updateSegmentLabels(result.segmentLabels, result.segments);
      // The split invalidated any redo branch, like merge does.
      markHistoryDirty();
      return result;
    } catch (e) {
      log.error("useTauriCommand", "splitSegment failed", { segmentId, error: String(e) });
      setStatusMessage(t("cmd.split.failed", String(e)));
      return null;
    }
  };

  /**
   * Snap a clicked model-local point to the nearest mesh vertex.
   * Returns the snapped vertex position + incident face for the lasso tool.
   */
  const manualRegionAddPoint = async (
    point: [number, number, number],
    faceIndex: number
  ): Promise<ManualPointResult | null> => {
    try {
      const result = await invoke<ManualPointResult>("manual_region_add_point", {
        point,
        faceIndex,
      });
      return result;
    } catch (e) {
      log.error("useTauriCommand", "manualRegionAddPoint failed", { point, error: String(e) });
      return null;
    }
  };

  /**
   * Finalize a manual lasso region from an ordered list of clicked points.
   * Backend snaps points, closes the loop, and assigns a fresh manual label.
   */
  const finalizeManualRegion = async (
    points: [number, number, number][],
    faceIndices: number[]
  ) => {
    // The backend emits `manual:*` segment-progress events during the close;
    // isLoading makes the Viewport ProgressBar overlay actually visible. The
    // overlay covers the backend compute AND the IPC payload parse (both happen
    // before the invoke promise resolves) — only the final React repaint lands
    // after it drops.
    setLoading(true);
    try {
      const result = await invoke<SegmentResult>("finalize_manual_region", {
        points,
        faceIndices,
      });
      updateSegmentLabels(result.segmentLabels, result.segments, result.faceColors);
      // Auto-select the freshly created region so the user immediately sees it
      // highlighted (cyan fill + yellow outline) instead of a silent "done".
      if (result.segments.length > 0) {
        const newLabel = result.segments.reduce((m, s) => Math.max(m, s.id), 0);
        setSelectedSegment(newLabel);
      }
      setStatusMessage(t("cmd.manual.done", result.segments.length));
      // REFUTE-driven (iteration 7, problem 3c1): surface a clear success popup
      // so the user knows the partition was created — the whole point of
      // partitioning is to then OPERATE on it (fill / inspect), not to have it
      // silently highlighted.
      setToast(t("cmd.manual.toast"));
      log.info("useTauriCommand", "finalizeManualRegion complete", {
        segments: result.segments.length,
      });
      return result.segments;
    } catch (e) {
      log.error("useTauriCommand", "finalizeManualRegion failed", { error: String(e) });
      setStatusMessage(t("cmd.manual.failed", String(e)));
    } finally {
      setLoading(false);
    }
  };

  /**
   * Smart auto-segmentation via Shape Diameter Function (semantic parts) is now
   * served by `autoSegmentV2({ type: "shapeDiameter", k })`. This hook only kept
   * the `auto_segment_smart` command alive; the command itself remains registered
   * in lib.rs as a stable legacy entry point but is no longer wired to the UI.
   */

  // 0.2.0-P1: open a .cym project. Clears the previous model's transient
  // state BEFORE setMeshData so stale seeds/overlays/selection can never leak
  // into the new model (setMeshData alone does not clear them).
  const loadProject = async (path: string) => {
    log.info("useTauriCommand", `loadProject("${path}")`);
    const t0 = performance.now();
    const data = await invoke<MeshData>("load_project", { path });
    const dt = (performance.now() - t0).toFixed(1);
    log.info("useTauriCommand", `loadProject returned in ${dt}ms`, {
      faces: data.faces.length / 3,
      segments: data.segments.length,
    });
    resetProjectTransientState();
    setMeshData(data);
    return data;
  };

  // 0.2.0-P1: save the current model as .cym. The project file carries the
  // paint, so saving marks the paint clean for exit-confirm / update-warn —
  // same semantics as exporting a 3MF (documented limitation: undoing after
  // save still counts as clean, matching the existing 3MF behaviour).
  const saveProject = async (path: string) => {
    log.info("useTauriCommand", `saveProject("${path}")`);
    await invoke("save_project", { path });
    markPaintExported();
  };

  const export3mf = async (path: string, selection?: ExportSelection) => {
    log.info("useTauriCommand", `export3mf("${path}")`, { selection });
    try {
      setStatusMessage(t("export.exporting"));
      const msg: string = await invoke("export_3mf_command", {
        path,
        selection: selection ?? null,
      });
      log.info("useTauriCommand", "export3mf complete", { msg });
      markPaintExported();
      setStatusMessage(t("export.complete"));
      setToast(t("export.success", msg.replace(/^Exported to /, "")));
      return msg;
    } catch (e) {
      log.error("useTauriCommand", "export3mf failed", { error: String(e) });
      setStatusMessage(t("export.failure", String(e)));
      setToast(t("export.failure", String(e)));
      throw e;
    }
  };

  /**
   * Export the current mesh as a `.obj` + `.mtl` carrying per-face colour.
   * No machine / process selection is involved, so the only argument is the
   * output path the save dialog produced.
   */
  const exportObj = async (path: string) => {
    log.info("useTauriCommand", `exportObj("${path}")`);
    try {
      setStatusMessage(t("export.exporting"));
      const msg: string = await invoke("export_obj_command", { path });
      log.info("useTauriCommand", "exportObj complete", { msg });
      markPaintExported();
      setStatusMessage(t("export.complete"));
      setToast(t("export.successObj", msg.replace(/^Exported to /, "")));
      return msg;
    } catch (e) {
      log.error("useTauriCommand", "exportObj failed", { error: String(e) });
      setStatusMessage(t("export.failure", String(e)));
      setToast(t("export.failure", String(e)));
      throw e;
    }
  };

  /**
   * Step the unified backend history one entry back. Returns the patch to apply,
   * or null on IPC failure. `applyHistoryResult` (MeshDisplay) renders it.
   */
  const undo = async (): Promise<HistoryResult | null> => {
    try {
      return await invoke<HistoryResult>("undo");
    } catch (e) {
      log.error("useTauriCommand", "undo failed", { error: String(e) });
      return null;
    }
  };

  /**
   * Step the unified backend history one entry forward.
   */
  const redo = async (): Promise<HistoryResult | null> => {
    try {
      return await invoke<HistoryResult>("redo");
    } catch (e) {
      log.error("useTauriCommand", "redo failed", { error: String(e) });
      return null;
    }
  };

  /**
   * Read the backend stack state (depths, can_undo/can_redo) without mutating it.
   * Useful to re-sync the toolbar after a model (re)load.
   */
  const historyState = async (): Promise<HistoryState | null> => {
    try {
      return await invoke<HistoryState>("history_state");
    } catch (e) {
      log.error("useTauriCommand", "history_state failed", { error: String(e) });
      return null;
    }
  };

  /**
   * Paint a single face into a manual segment.
   * Called per-face during segment brush drag. `strokeId` groups the whole drag
   * into one backend undo entry (segment brush is a drag, not a click).
   * Returns { faceId, color, segmentLabel } for incremental GPU color update.
   */
  const paintSegmentFace = async (
    faceId: number,
    segmentLabel?: number,
    strokeId?: number
  ): Promise<{ faceId: number; color: number[]; segmentLabel: number } | null> => {
    try {
      const result = await invoke<{
        faceId: number;
        color: number[];
        segmentLabel: number;
      }>("paint_segment_face", {
        faceId,
        segmentLabel: segmentLabel ?? null,
        strokeId: strokeId ?? null,
      });
      return result;
    } catch (e) {
      log.error("useTauriCommand", "paintSegmentFace failed", { faceId, error: String(e) });
      return null;
    }
  };

  /**
   * Finalize a manual segment after drag ends.
   * Rebuilds segment metadata (face counts, names, colors) in store.
   */
  const finalizeSegment = async (segmentLabel: number) => {
    try {
      const result = await invoke<{
        segments: any[];
        segmentLabels: number[];
      }>("finalize_segment", { segmentLabel });

      updateSegmentLabels(result.segmentLabels, result.segments);
      setStatusMessage(t("cmd.finalizeSegment.done", result.segments.length));
      log.info("useTauriCommand", "finalizeSegment complete", {
        segments: result.segments.length,
      });
    } catch (e) {
      log.error("useTauriCommand", "finalizeSegment failed", { error: String(e) });
    }
  };

  /**
   * Re-run any algorithm on a single region, splitting it into sub-regions.
   * Returns the refreshed SegmentResult so the panel can update labels; the
   * backend records an undo op, so this is reversible like a split.
   */
  const resegmentRegion = async (
    label: number,
    algorithm: SegmentationAlgorithm
  ): Promise<SegmentResult> => {
    log.info("useTauriCommand", `resegmentRegion(${label}, ${algorithm.type})`);
    try {
      setStatusMessage(t("cmd.resegment.working"));
      const result = await invoke<SegmentResult>("resegment_region", { label, algorithm });
      updateSegmentLabels(result.segmentLabels, result.segments);
      setStatusMessage(t("cmd.resegment.done", result.segments.length));
      return result;
    } catch (e) {
      log.error("useTauriCommand", "resegmentRegion failed", { error: String(e) });
      setStatusMessage(t("cmd.resegment.failed", tErr(e)));
      throw e;
    }
  };

  /**
   * Seeded watershed segmentation (iteration 50). `seeds` are the user-placed
   * points (one region each); the backend grows them by feature-edge barriers +
   * geodesic nearest-seed Voronoi, filling any un-seeded patch from the
   * geodesic-nearest seed across barriers. Returns the refreshed SegmentResult.
   */
  const seedGrow = async (
    seeds: SeedPoint[],
    barrierDeg: number,
    optimizer: boolean
  ): Promise<SegmentResult> => {
    log.info("useTauriCommand", `seedGrow(${seeds.length} seeds, barrier=${barrierDeg}°)`);
    try {
      setStatusMessage(t("cmd.seed.working"));
      // Map the JS SeedPoint shape ({x,y,z,faceIndex}) to the Rust SeedInput
      // ({point, face_index}) the command expects. Tauri 2 only converts
      // top-level invoke keys (camelCase↔snake_case); nested struct fields
      // must already match the Rust side, so we spell `face_index` here.
      const payload = seeds.map((s) => ({ point: [s.x, s.y, s.z], face_index: s.faceIndex }));
      const result = await invoke<SegmentResult>("seed_grow", { seeds: payload, barrierDeg, optimizer });
      updateSegmentLabels(result.segmentLabels, result.segments, result.faceColors);
      setStatusMessage(t("seed.done", result.segments.length));
      return result;
    } catch (e) {
      log.error("useTauriCommand", "seedGrow failed", { error: String(e) });
      setStatusMessage(t("cmd.seed.failed", tErr(e)));
      throw e;
    }
  };

  /**
   * Suggest seed locations for the seeded-watershed tool (iteration 52). The
   * backend runs weighted FPS over face centroids and returns advisory points;
   * the UI renders them as ghost markers the user accepts (click) or ignores.
   * Returns the suggested SeedPoints for the store.
   */
  const recommendSeeds = async (
    count: number,
    curvature: number,
    concavity: number
  ): Promise<SeedPoint[]> => {
    log.info(
      "useTauriCommand",
      `recommendSeeds(${count}, curv=${curvature}, conc=${concavity})`
    );
    try {
      setStatusMessage(t("cmd.suggest.working"));
      // Backend returns { point: [x,y,z], faceIndex } — map to the JS SeedPoint
      // shape ({x,y,z,faceIndex}). Tauri 2 only converts top-level invoke keys,
      // so nested field names already match the Rust side (camelCase here).
      const raw = await invoke<Array<{ point: [number, number, number]; faceIndex: number }>>(
        "recommend_seeds",
        { count, curvature, concavity }
      );
      const seeds: SeedPoint[] = raw.map((r) => ({
        x: r.point[0],
        y: r.point[1],
        z: r.point[2],
        faceIndex: r.faceIndex,
      }));
      setStatusMessage(t("cmd.suggest.done", seeds.length));
      return seeds;
    } catch (e) {
      log.error("useTauriCommand", "recommendSeeds failed", { error: String(e) });
      setStatusMessage(t("cmd.suggest.failed", String(e)));
      throw e;
    }
  };

  /**
   * Layer 1 planar-region detection (`docs/09`): find the mesh's continuous
   * flat patches and return them as advisory seed suggestions. Each region
   * carries a representative `seed` (a `SeedPoint` to drop into the ghost set)
   * and `boundaryEdges` (3D outline segments). Read-only; nothing is committed
   * until the user accepts a seed into `seed_grow`.
   */
  const detectPlanarRegions = async (
    angleThresholdDeg: number,
    distThrFactor: number,
    minRegionFaces: number
  ): Promise<PlanarRegion[]> => {
    log.info(
      "useTauriCommand",
      `detectPlanarRegions(angle=${angleThresholdDeg}°, distFactor=${distThrFactor}, min=${minRegionFaces})`
    );
    try {
      setStatusMessage(t("cmd.planar.working"));
      const raw = await invoke<
        Array<{
          plane: [number, number, number, number];
          faceCount: number;
          seed: { point: [number, number, number]; faceIndex: number };
          boundaryEdges: number[][][];
          faceIndices: number[];
        }>
      >("detect_planar_regions", {
        angleThresholdDeg,
        distThrFactor,
        minRegionFaces,
      });
      const regions: PlanarRegion[] = (raw ?? []).map((r) => ({
        plane: r.plane,
        faceCount: r.faceCount,
        seed: { x: r.seed.point[0], y: r.seed.point[1], z: r.seed.point[2], faceIndex: r.seed.faceIndex },
        boundaryEdges: r.boundaryEdges,
        faceIndices: r.faceIndices,
      }));
      setStatusMessage(t("cmd.planar.done", regions.length));
      return regions;
    } catch (e) {
      log.error("useTauriCommand", "detectPlanarRegions failed", { error: String(e) });
      setStatusMessage(t("cmd.planar.failed", String(e)));
      return [];
    }
  };

  /// Layer 3 (MultiView 3→2→3) detection. Returns consensus clusters as advisory
  /// seed suggestions; nothing reaches `seed_grow` until the user accepts one.
  const detectMultiViewRegions = async (
    viewCount: number,
    angleThresholdDeg: number,
    minRegionFaces: number,
    matchThreshold: number
  ): Promise<MultiViewRegion[]> => {
    log.info(
      "useTauriCommand",
      `detectMultiViewRegions(views=${viewCount}, angle=${angleThresholdDeg}°, min=${minRegionFaces}, match=${matchThreshold})`
    );
    try {
      setStatusMessage(t("cmd.multiview.working"));
      const raw = await invoke<
        Array<{
          faceCount: number;
          seed: { point: [number, number, number]; faceIndex: number };
          boundaryEdges: number[][][];
          faceIndices: number[];
        }>
      >("detect_multiview_regions", {
        viewCount,
        angleThresholdDeg,
        minRegionFaces,
        matchThreshold,
      });
      const regions: MultiViewRegion[] = (raw ?? []).map((r) => ({
        faceCount: r.faceCount,
        seed: { x: r.seed.point[0], y: r.seed.point[1], z: r.seed.point[2], faceIndex: r.seed.faceIndex },
        boundaryEdges: r.boundaryEdges,
        faceIndices: r.faceIndices,
      }));
      setStatusMessage(t("cmd.multiview.done", regions.length));
      return regions;
    } catch (e) {
      log.error("useTauriCommand", "detectMultiViewRegions failed", { error: String(e) });
      setStatusMessage(t("cmd.multiview.failed", String(e)));
      return [];
    }
  };

  /// Layer 2 (cross-section / ray marching, docs/09) detection. Returns feature
  /// cross-sections as a *visual-only* evidence overlay (contour lines +
  /// inside/outside confidence). Nothing reaches `seed_grow` — a slice is a
  /// plane, not a face (docs/09 Layer 2 = "不裁决").
  const detectCrossSectionRegions = async (
    planesPerAxis: number,
    featureThreshold: number
  ): Promise<CrossSectionRegion[]> => {
    log.info(
      "useTauriCommand",
      `detectCrossSectionRegions(planesPerAxis=${planesPerAxis}, featureThreshold=${featureThreshold})`
    );
    try {
      setStatusMessage(t("cmd.crossSection.working"));
      const raw = await invoke<
        Array<{
          plane: number[];
          axis: number;
          position: number;
          areaMetric: number;
          winding: number;
          boundaryEdges: number[][][];
        }>
      >("detect_cross_section_features", {
        planesPerAxis,
        featureThreshold,
      });
      const regions: CrossSectionRegion[] = (raw ?? []).map((r) => ({
        plane: r.plane,
        axis: r.axis,
        position: r.position,
        areaMetric: r.areaMetric,
        winding: r.winding,
        boundaryEdges: r.boundaryEdges,
      }));
      setStatusMessage(t("cmd.crossSection.done", regions.length));
      return regions;
    } catch (e) {
      log.error("useTauriCommand", "detectCrossSectionRegions failed", { error: String(e) });
      setStatusMessage(t("cmd.crossSection.failed", String(e)));
      return [];
    }
  };

  /// Layer 5 (docs/10) eye-region semantic detection. Given a user-supplied ROI
  /// (the face indices of a partition that bounds an eye), the backend
  /// classifies each ROI face as globe / sclera / eyelid / socket and returns
  /// the sub-regions with their boundary outlines. Read-only; nothing is
  /// committed to the mesh — it is a visual evidence overlay like Layer 1/2/3.
  const detectEyeRegions = async (roiFaces: number[]): Promise<EyeRegion[]> => {
    log.info("useTauriCommand", `detectEyeRegions(roi=${roiFaces.length} faces)`);
    try {
      setStatusMessage(t("cmd.eye.working"));
      const raw = await invoke<
        Array<{
          semantic: EyeRegion["semantic"];
          faceIndices: number[];
          boundaryEdges: number[][][];
          center: [number, number, number];
          confidence: number;
        }>
      >("detect_eye_regions", { roiFaces });
      const regions: EyeRegion[] = (raw ?? []).map((r) => ({
        semantic: r.semantic,
        faceIndices: r.faceIndices,
        boundaryEdges: r.boundaryEdges,
        center: r.center,
        confidence: r.confidence,
      }));
      // Diagnostic breakdown: a "only 1 region" report is triaged by which
      // labels survive — only Eyelid ⇒ ROI was the wrong partition; Globe +
      // Sclera present ⇒ the split worked and the overlay is just subtle.
      const tally: Record<string, number> = {};
      for (const r of regions) {
        tally[r.semantic] = (tally[r.semantic] ?? 0) + 1;
      }
      const parts = ["globe", "sclera", "eyelid", "socket"]
        .map((k) => `${k}:${tally[k] ?? 0}`)
        .join(" ");
      setStatusMessage(t("cmd.eye.done", regions.length, parts));
      return regions;
    } catch (e) {
      log.error("useTauriCommand", "detectEyeRegions failed", { error: String(e) });
      setStatusMessage(t("cmd.eye.failed", String(e)));
      return [];
    }
  };

  /// Stage 1: global eye detection without a user-supplied ROI. Scans the whole
  /// mesh for symmetric eye-like bumps and returns EyeRegions that can be fed
  /// straight into the fusion path.
  const detectEyeRegionsAuto = async (): Promise<EyeRegion[]> => {
    log.info("useTauriCommand", "detectEyeRegionsAuto()");
    try {
      setStatusMessage(t("cmd.eyeAuto.working"));
      const raw = await invoke<
        Array<{
          semantic: EyeRegion["semantic"];
          faceIndices: number[];
          boundaryEdges: number[][][];
          center: [number, number, number];
          confidence: number;
        }>
      >("detect_eye_regions_auto");
      const regions: EyeRegion[] = (raw ?? []).map((r) => ({
        semantic: r.semantic,
        faceIndices: r.faceIndices,
        boundaryEdges: r.boundaryEdges,
        center: r.center,
        confidence: r.confidence,
      }));
      const tally: Record<string, number> = {};
      for (const r of regions) {
        tally[r.semantic] = (tally[r.semantic] ?? 0) + 1;
      }
      const parts = ["globe", "sclera", "eyelid", "socket"]
        .map((k) => `${k}:${tally[k] ?? 0}`)
        .join(" ");
      setStatusMessage(t("cmd.eyeAuto.done", regions.length, parts));
      return regions;
    } catch (e) {
      log.error("useTauriCommand", "detectEyeRegionsAuto failed", { error: String(e) });
      setStatusMessage(t("cmd.eyeAuto.failed", String(e)));
      return [];
    }
  };

  /**
   * Layer 4 (docs/09): fuse the planar (Layer 1) + multiview (Layer 3) region
   * *memberships* into one partition via edge-level majority vote and commit it
   * to the mesh — the fix for "the algorithms sketch useful regions but the
   * real partition is still just seeds". The backend runs both detectors with
   * the panel's defaults and fuses their face membership; only the fusion knobs
   * are exposed here.
   */
  const fuseSegmentation = async (
    cutThreshold: number = 1,
    minRegionFaces: number = 0,
    // 2° matches the SeedPanel slider default: sculpted figures spread their
    // part folds over sub-5° edges, so 15° here produced one giant region.
    dihedralDeg: number = 2,
    eyeFaceIndices?: number[][]
  ): Promise<SegmentResult> => {
    log.info(
      "useTauriCommand",
      `fuseSegmentation(cutThreshold=${cutThreshold}, minRegionFaces=${minRegionFaces}, dihedralDeg=${dihedralDeg}, eyeSets=${eyeFaceIndices?.length ?? 0})`
    );
    try {
      setStatusMessage(t("cmd.fuse.working"));
      // The backend parameter is `eye_face_indices` (snake_case). Tauri 2
      // converts top-level invoke keys but the nested `Vec<Vec<u32>>` shape is
      // already what serde expects, so we pass through as-is. When the caller
      // has no eye regions (most of the time) we send an empty array so the
      // backend falls into the original 3-channel vote.
      const result = await invoke<SegmentResult>("fuse_segmentation", {
        cutThreshold,
        minRegionFaces,
        dihedralDeg,
        eyeFaceIndices: eyeFaceIndices ?? [],
      });
      updateSegmentLabels(result.segmentLabels, result.segments, result.faceColors);
      setStatusMessage(t("cmd.fuse.done", result.segments.length));
      return result;
    } catch (e) {
      log.error("useTauriCommand", "fuseSegmentation failed", { error: String(e) });
      setStatusMessage(t("cmd.fuse.failed", tErr(e)));
      throw e;
    }
  };

  // 0.2.0-P2 spike: one OBJ per connected same-colour shell (open shells,
  // no hole closing) + manifest — the deliverable of the split-export
  // feasibility question; slicer behaviour is verified by hand.
  const exportSplitByColor = async (dirPath: string) => {
    setStatusMessage(t("status.exportingSplit"));
    const parts = await invoke<string[]>("export_split_by_color_command", {
      dirPath,
    });
    setStatusMessage(t("status.splitExported", parts.length));
    return parts;
  };

  return {
    loadModel,
    loadProject,
    saveProject,
    exportSplitByColor,
    export3mf,
    exportObj,
    paintSegmentFace,
    finalizeSegment,
    renameSegment,
    mergeSegments,
    splitSegment,
    resegmentRegion,
    seedGrow,
    recommendSeeds,
    detectPlanarRegions,
    detectMultiViewRegions,
    detectCrossSectionRegions,
    detectEyeRegions,
    detectEyeRegionsAuto,
    fuseSegmentation,
    resetSegmentation,
    manualRegionAddPoint,
    finalizeManualRegion,
    undo,
    redo,
    historyState,
  };
}
