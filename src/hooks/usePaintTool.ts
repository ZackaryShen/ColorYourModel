import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../store/appStore";
import { PaintResult, PaintTool } from "../types/mesh";
import { log } from "../utils/logger";

export function usePaintTool() {
  const activeTool = useAppStore((s) => s.activeTool);
  const brushRadius = useAppStore((s) => s.brushRadius);
  const brushStrength = useAppStore((s) => s.brushStrength);
  const brushFalloff = useAppStore((s) => s.brushFalloff);
  const currentColor = useAppStore((s) => s.currentColor);
  const segmentView = useAppStore((s) => s.segmentView);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const setSelectedSegment = useAppStore((s) => s.setSelectedSegment);
  const setSegmentView = useAppStore((s) => s.setSegmentView);

  const fillSegment = async (
    faceId: number
  ): Promise<PaintResult | null> => {
    const meshData = useAppStore.getState().meshData;
    if (!meshData || !meshData.segmentLabels.length) return null;
    const segmentId = meshData.segmentLabels[faceId];
    if (segmentId === undefined) return null;

    // Select this segment visually
    setSelectedSegment(segmentId);
    log.info("usePaintTool", `fillSegment: face ${faceId} → segment ${segmentId}`);

    try {
      const result = await invoke<PaintResult>("fill_segment_paint", {
        segmentId,
        color: currentColor,
      });
      log.debug("usePaintTool", "fillSegment result", { faces: result.updatedFaces.length });
      // Switch to paint view so user can see the fill result
      setSegmentView(false);
      setStatusMessage(`Filled segment ${segmentId} (${result.updatedFaces.length} faces)`);
      return result;
    } catch (e) {
      log.error("usePaintTool", "fillSegment failed", { segmentId, error: String(e) });
      setStatusMessage(`Segment fill error: ${e}`);
      return null;
    }
  };

  const paintFace = async (
    faceId: number
  ): Promise<PaintResult | null> => {
    // In segment view mode, fill the entire segment instead of normal paint
    if (segmentView) {
      return fillSegment(faceId);
    }

    log.debug("usePaintTool", `paintFace(${faceId})`, { tool: activeTool, color: currentColor });
    try {
      let result: PaintResult;

      switch (activeTool) {
        case PaintTool.Fill:
          result = await invoke<PaintResult>("fill_paint", {
            faceId,
            color: currentColor,
          });
          break;

        case PaintTool.Brush:
          result = await invoke<PaintResult>("brush_paint", {
            centerFace: faceId,
            radius: brushRadius,
            strength: brushStrength,
            falloffMode: brushFalloff,
            color: currentColor,
          });
          break;

        case PaintTool.Spray:
          result = await invoke<PaintResult>("spray_paint", {
            centerFace: faceId,
            radius: brushRadius,
            strength: brushStrength,
            color: currentColor,
            density: 50,
          });
          break;

        case PaintTool.SmartBrush:
          result = await invoke<PaintResult>("smart_brush_paint", {
            centerFace: faceId,
            radius: brushRadius,
            strength: brushStrength,
            falloffMode: brushFalloff,
            color: currentColor,
          });
          break;

        case PaintTool.Eraser:
          result = await invoke<PaintResult>("erase_paint", {
            centerFace: faceId,
            radius: brushRadius,
          });
          break;

        case PaintTool.Eyedropper: {
          const color = await invoke<[number, number, number, number]>(
            "pick_color",
            { faceId }
          );
          useAppStore.getState().setCurrentColor(color);
          setStatusMessage(`Picked color: rgb(${color[0]},${color[1]},${color[2]})`);
          return null;
        }

        default:
          return null;
      }

      log.debug("usePaintTool", "paintFace result", {
        faces: result.updatedFaces.length,
      });
      return result;
    } catch (e) {
      log.error("usePaintTool", "Paint command failed", { faceId, tool: activeTool, error: String(e) });
      setStatusMessage(`Paint error: ${e}`);
      return null;
    }
  };

  return { paintFace, fillSegment };
}
