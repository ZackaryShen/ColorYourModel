import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../store/appStore";
import { MeshData } from "../types/mesh";
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
      const result = await invoke<{ segments: any[]; segmentLabels: number[] }>(
        "auto_segment",
        { angleThreshold }
      );
      const dt = (performance.now() - t0).toFixed(1);

      log.info("useTauriCommand", `autoSegment returned in ${dt}ms`, {
        segmentCount: result.segments.length,
        labelCount: result.segmentLabels.length,
      });

      // Update both segment metadata AND per-face labels in meshData
      updateSegmentLabels(result.segmentLabels, result.segments);
      setStatusMessage(`Segmented into ${result.segments.length} regions`);
      return result.segments;
    } catch (e) {
      log.error("useTauriCommand", "autoSegment failed", { error: String(e) });
      setStatusMessage(`Segment failed: ${e}`);
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

  return { loadModel, autoSegment, export3mf, paintSegmentFace, finalizeSegment };
}
