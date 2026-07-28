import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../store/appStore";
import { MeshData, ManualPointResult, SegmentResult } from "../types/mesh";
import { log } from "../utils/logger";

export function useTauriCommand() {
  const setMeshData = useAppStore((s) => s.setMeshData);
  const updateSegmentLabels = useAppStore((s) => s.updateSegmentLabels);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);

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
    point: [number, number, number]
  ): Promise<ManualPointResult | null> => {
    try {
      const result = await invoke<ManualPointResult>("manual_region_add_point", { point });
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
  const finalizeManualRegion = async (points: [number, number, number][]) => {
    try {
      const result = await invoke<SegmentResult>("finalize_manual_region", { points });
      updateSegmentLabels(result.segmentLabels, result.segments, result.faceColors);
      setStatusMessage(`手动分区完成（共 ${result.segments.length} 个区域）`);
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
   * Undo the last finalized manual (lasso) region.
   * Backend restores affected faces to their pre-finalize state and returns the
   * updated segment metadata + face colors for repaint.
   */
  const manualRegionUndo = async (): Promise<SegmentResult | null> => {
    try {
      const result = await invoke<SegmentResult>("manual_region_undo");
      updateSegmentLabels(result.segmentLabels, result.segments, result.faceColors);
      setStatusMessage("已撤销上一个手动分区");
      log.info("useTauriCommand", "manualRegionUndo complete", {
        segments: result.segments.length,
      });
      return result;
    } catch (e) {
      log.error("useTauriCommand", "manualRegionUndo failed", { error: String(e) });
      setStatusMessage(`撤销失败：${e}`);
      return null;
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
   * Paint a single face into a manual segment.
   * Called per-face during segment brush drag.
   * Returns { faceId, color, segmentLabel } for incremental GPU color update.
   */
  const paintSegmentFace = async (
    faceId: number,
    segmentLabel?: number
  ): Promise<{ faceId: number; color: number[]; segmentLabel: number } | null> => {
    try {
      const result = await invoke<{
        faceId: number;
        color: number[];
        segmentLabel: number;
      }>("paint_segment_face", {
        faceId,
        segmentLabel: segmentLabel ?? null,
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
    manualRegionUndo,
  };
}
