import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../store/appStore";
import { MeshData, ManualPointResult, SegmentResult, HistoryResult, HistoryState } from "../types/mesh";
import { log } from "../utils/logger";

export function useTauriCommand() {
  const setMeshData = useAppStore((s) => s.setMeshData);
  const updateSegmentLabels = useAppStore((s) => s.updateSegmentLabels);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const setSelectedSegment = useAppStore((s) => s.setSelectedSegment);
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

  const autoSegment = async (angleThreshold: number = 30.0) => {
    log.info("useTauriCommand", `autoSegment(${angleThreshold})`);
    try {
      setStatusMessage("Segmenting...");
      const t0 = performance.now();
      const result = await invoke<SegmentResult>("auto_segment", { angleThreshold });
      const dt = (performance.now() - t0).toFixed(1);

      log.info("useTauriCommand", `autoSegment returned in ${dt}ms`, {
        segmentCount: result.segments.length,
        labelCount: result.segmentLabels.length,
      });

      // Update both segment metadata AND per-face labels in meshData
      updateSegmentLabels(result.segmentLabels, result.segments, result.faceColors);
      setStatusMessage(`Segmented into ${result.segments.length} regions`);
      return result.segments;
    } catch (e) {
      log.error("useTauriCommand", "autoSegment failed", { error: String(e) });
      setStatusMessage(`Segment failed: ${e}`);
      throw e;
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
   * Smart auto-segmentation via Shape Diameter Function (semantic parts).
   * `k = 0` auto-estimates cluster count from SDF peaks.
   */
  const autoSegmentSmart = async (k: number = 0) => {
    log.info("useTauriCommand", `autoSegmentSmart(${k})`);
    try {
      setStatusMessage("智能分区中（SDF）...");
      const result = await invoke<SegmentResult>("auto_segment_smart", { k });
      updateSegmentLabels(result.segmentLabels, result.segments, result.faceColors);
      setStatusMessage(`智能分区完成：${result.segments.length} 个零件`);
      log.info("useTauriCommand", "autoSegmentSmart done", {
        segmentCount: result.segments.length,
      });
      return result.segments;
    } catch (e) {
      log.error("useTauriCommand", "autoSegmentSmart failed", { error: String(e) });
      setStatusMessage(`智能分区失败：${e}`);
      throw e;
    }
  };

  const export3mf = async (path: string) => {
    log.info("useTauriCommand", `export3mf("${path}")`);
    try {
      setStatusMessage("Exporting...");
      await invoke("export_3mf_command", { path });
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

  return {
    loadModel,
    autoSegment,
    autoSegmentSmart,
    export3mf,
    paintSegmentFace,
    finalizeSegment,
    manualRegionAddPoint,
    finalizeManualRegion,
    undo,
    redo,
    historyState,
  };
}
