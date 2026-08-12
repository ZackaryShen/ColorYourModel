import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../store/appStore";
import { MeshData, ManualPointResult, Segment, SegmentResult, HistoryResult, HistoryState, SeedPoint } from "../types/mesh";
import type { ExportSelection } from "../types/export";
import type { SegmentationAlgorithm, SplitMethod, SplitResult } from "../types/segment";
import { log } from "../utils/logger";

export function useTauriCommand() {
  const setMeshData = useAppStore((s) => s.setMeshData);
  const updateSegmentLabels = useAppStore((s) => s.updateSegmentLabels);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const setSelectedSegment = useAppStore((s) => s.setSelectedSegment);
  const setSegmentMetadata = useAppStore((s) => s.setSegmentMetadata);
  const markHistoryDirty = useAppStore((s) => s.markHistoryDirty);
  const setToast = useAppStore((s) => s.setToast);

  const loadModel = async (path: string) => {
    log.info("useTauriCommand", `loadModel("${path}")`);
    try {
      setStatusMessage("Loading model...");
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
      setStatusMessage(`Loaded: ${data.faceCount} faces`);
      return data;
    } catch (e) {
      log.error("useTauriCommand", "loadModel failed", { path, error: String(e) });
      setStatusMessage(`Load failed: ${e}`);
      throw e;
    }
  };

  /**
   * Unified multi-algorithm auto-segmentation entry point. The UI sends a single
   * `algorithm` enum (dihedral | shapeDiameter | curvatureKMeans) assembled by
   * `buildAlgorithm`; the backend dispatches via `run_segmentation`. Replaces the
   * old per-algorithm `autoSegment` / `autoSegmentSmart` hooks (REFUTE: avoid N
   * near-identical commands and the configuration drift that caused).
   *
   * `preserveManual` keeps hand-drawn regions (labels >= MANUAL_SEGMENT_OFFSET)
   * alive across a re-run: the algorithm still claims every face, then the
   * backend paints the manual labels back on top. It defaults to true here AND
   * in Rust so that any future call site that forgets the argument still fails
   * safe. We deliberately did NOT gate this behind a confirm dialog — a dialog
   * on the panel would not cover Toolbar.tsx's segment-on-import path, so manual
   * regions would still vanish silently there.
   */
  const autoSegmentV2 = async (
    algorithm: SegmentationAlgorithm,
    preserveManual: boolean = true
  ) => {
    log.info("useTauriCommand", `autoSegmentV2(${algorithm.type})`, { preserveManual });
    try {
      setStatusMessage("智能分区中…");
      const t0 = performance.now();
      const result = await invoke<SegmentResult>("auto_segment_v2", {
        algorithm,
        preserveManual,
      });
      const dt = (performance.now() - t0).toFixed(1);

      log.info("useTauriCommand", `autoSegmentV2 returned in ${dt}ms`, {
        segmentCount: result.segments.length,
        labelCount: result.segmentLabels.length,
      });

      // Update both segment metadata AND per-face labels in meshData. The
      // backend no longer returns faceColors for auto-segmentation (it never
      // changes them), so colours are left untouched — also avoids a multi-MB
      // IPC payload and a full repaint (see commands/segment.rs REFUTE major-5).
      updateSegmentLabels(result.segmentLabels, result.segments);
      setStatusMessage(`分区完成：${result.segments.length} 个区域`);
      return result.segments;
    } catch (e) {
      log.error("useTauriCommand", "autoSegmentV2 failed", { error: String(e) });
      setStatusMessage(`分区失败：${e}`);
      throw e;
    }
  };

  /**
   * Wipe the current segmentation AND all face paint, returning the mesh to its
   * freshly-loaded "uncoloured" state (iteration 57, B2). Unlike `autoSegmentV2`
   * this DOES return the (neutral) face-colour buffer so the frontend repaints
   * back to grey in one shot — no model reload needed.
   */
  const resetSegmentation = async (): Promise<void> => {
    try {
      setStatusMessage("正在重置分区…");
      const result = await invoke<SegmentResult>("reset_segmentation");
      // faceColors length matches meshData.faceColors, so the store guard passes
      // and the canvas repaints to neutral grey.
      updateSegmentLabels(result.segmentLabels, result.segments, result.faceColors);
      setStatusMessage("已重置：分区与上色均已清除");
    } catch (e) {
      log.error("useTauriCommand", "resetSegmentation failed", { error: String(e) });
      setStatusMessage(`重置失败：${e}`);
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
      setStatusMessage(`重命名失败：${e}`);
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
      setStatusMessage(`合并失败：${e}`);
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
      setStatusMessage(`拆分失败：${e}`);
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
      setStatusMessage(`手动分区完成（共 ${result.segments.length} 个区域）`);
      // REFUTE-driven (iteration 7, problem 3c1): surface a clear success popup
      // so the user knows the partition was created — the whole point of
      // partitioning is to then OPERATE on it (fill / inspect), not to have it
      // silently highlighted.
      setToast("添加分区成功");
      log.info("useTauriCommand", "finalizeManualRegion complete", {
        segments: result.segments.length,
      });
      return result.segments;
    } catch (e) {
      log.error("useTauriCommand", "finalizeManualRegion failed", { error: String(e) });
      setStatusMessage(`手动分区失败：${e}`);
    }
  };

  /**
   * Smart auto-segmentation via Shape Diameter Function (semantic parts) is now
   * served by `autoSegmentV2({ type: "shapeDiameter", k })`. This hook only kept
   * the `auto_segment_smart` command alive; the command itself remains registered
   * in lib.rs as a stable legacy entry point but is no longer wired to the UI.
   */

  const export3mf = async (path: string, selection?: ExportSelection) => {
    log.info("useTauriCommand", `export3mf("${path}")`, { selection });
    try {
      setStatusMessage("Exporting...");
      await invoke("export_3mf_command", { path, selection: selection ?? null });
      log.info("useTauriCommand", "export3mf complete");
      setStatusMessage("Export complete");
    } catch (e) {
      log.error("useTauriCommand", "export3mf failed", { error: String(e) });
      setStatusMessage(`Export failed: ${e}`);
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
      setStatusMessage(`Segment created (${result.segments.length} total regions)`);
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
      setStatusMessage("区域内再分区中…");
      const result = await invoke<SegmentResult>("resegment_region", { label, algorithm });
      updateSegmentLabels(result.segmentLabels, result.segments);
      setStatusMessage(`再分区完成：${result.segments.length} 个区域`);
      return result;
    } catch (e) {
      log.error("useTauriCommand", "resegmentRegion failed", { error: String(e) });
      setStatusMessage(`再分区失败：${e}`);
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
      setStatusMessage("种子生长分区中…");
      // Map the JS SeedPoint shape ({x,y,z,faceIndex}) to the Rust SeedInput
      // ({point, face_index}) the command expects. Tauri 2 only converts
      // top-level invoke keys (camelCase↔snake_case); nested struct fields
      // must already match the Rust side, so we spell `face_index` here.
      const payload = seeds.map((s) => ({ point: [s.x, s.y, s.z], face_index: s.faceIndex }));
      const result = await invoke<SegmentResult>("seed_grow", { seeds: payload, barrierDeg, optimizer });
      updateSegmentLabels(result.segmentLabels, result.segments, result.faceColors);
      setStatusMessage(`种子分区完成：${result.segments.length} 个区域`);
      return result;
    } catch (e) {
      log.error("useTauriCommand", "seedGrow failed", { error: String(e) });
      setStatusMessage(`种子分区失败：${e}`);
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
      setStatusMessage("正在推荐种子点位…");
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
      setStatusMessage(`已推荐 ${seeds.length} 个候选种子（点击接受）`);
      return seeds;
    } catch (e) {
      log.error("useTauriCommand", "recommendSeeds failed", { error: String(e) });
      setStatusMessage(`推荐种子失败：${e}`);
      throw e;
    }
  };

  return {
    loadModel,
    autoSegmentV2,
    export3mf,
    paintSegmentFace,
    finalizeSegment,
    renameSegment,
    mergeSegments,
    splitSegment,
    resegmentRegion,
    seedGrow,
    recommendSeeds,
    resetSegmentation,
    manualRegionAddPoint,
    finalizeManualRegion,
    undo,
    redo,
    historyState,
  };
}
