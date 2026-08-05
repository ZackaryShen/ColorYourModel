import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../store/appStore";
import { PaintResult, PaintTool } from "../types/mesh";
import { log } from "../utils/logger";

/** A clicked partition covering more than this share of the model is treated as
 *  "the whole model", so filling it would be indistinguishable from flooding
 *  everything. Above the threshold the fill degrades to a local, bounded blob
 *  (iteration 18, Issue 1).
 *
 * Manual regions (lasso/segment brush) use a slightly more permissive 0.95
 *  because the user explicitly drew them and expects Fill to cover the entire
 *  drawn area (iteration 22 M6 guard: prevent one-click half-model flood while
 *  still allowing large intentional regions). */
export const WHOLE_SEGMENT_MAX_SHARE = 0.8;
export const MANUAL_REGION_MAX_SHARE = 0.95;

/// Labels >= this value are manually-created regions (lasso / freehand).
/// These should ALWAYS be filled as whole partitions — the user explicitly
/// drew them and expects fill to cover the entire region (iteration 21 fix).
export const MANUAL_SEGMENT_OFFSET = 100_000;

export function usePaintTool() {
  const activeTool = useAppStore((s) => s.activeTool);
  const brushRadius = useAppStore((s) => s.brushRadius);
  const brushStrength = useAppStore((s) => s.brushStrength);
  const brushFalloff = useAppStore((s) => s.brushFalloff);
  const currentColor = useAppStore((s) => s.currentColor);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const setSelectedSegment = useAppStore((s) => s.setSelectedSegment);

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
      // NOTE: we deliberately do NOT switch views here. The user clicked a
      // partition to fill it in the CURRENT view (iteration 7, problem 3c3) —
      // switching to paint view mid-click was the confusing part. The fill
      // result is repainted in-place and the segment stays selected/highlighted.
      // Show the actual fill color (hex) in the status bar (iteration 28): the
      // dominant cause of "fill turns black" is a black currentColor (AMS
      // palette #2 / persisted from a previous session), which the user often
      // doesn't notice is selected. Making it explicit on every fill removes
      // the ambiguity without a warning toast (black is a legitimate AMS color).
      const c = useAppStore.getState().currentColor;
      const hex = `#${c[0].toString(16).padStart(2, "0")}${c[1].toString(16).padStart(2, "0")}${c[2].toString(16).padStart(2, "0")}`.toUpperCase();
      setStatusMessage(`已填充分区 ${segmentId}（${result.updatedFaces.length} 个面）颜色 ${hex}`);
      return result;
    } catch (e) {
      log.error("usePaintTool", "fillSegment failed", { segmentId, error: String(e) });
      setStatusMessage(`分区填充失败：${e}`);
      return null;
    }
  };

  const paintFace = async (
    faceId: number,
    opts?: { wholeRegion?: boolean }
  ): Promise<PaintResult | null> => {
    log.debug("usePaintTool", `paintFace(${faceId})`, { tool: activeTool, color: currentColor });
    try {
      let result: PaintResult;

      switch (activeTool) {
        case PaintTool.Fill: {
          // "智能区分" fill (iteration 18, Issue 1 — user-confirmed behaviour):
          //   • model split into several REAL partitions and the clicked one is
          //     not the whole model → fill that WHOLE partition;
          //   • unsegmented, single-partition, or the clicked partition covers
          //     (almost) the entire model → fall back to a LOCAL bounded fill of
          //     radius `brushRadius`.
          //
          //   • MANUAL region (label >= MANUAL_SEGMENT_OFFSET, i.e. lasso/
          //     freehand) → ALWAYS fill the whole partition, regardless of
          //     share or realSegs count (iteration 21 fix: user explicitly drew
          //     this region and expects fill to cover it entirely).
          //
          //   • **Hover-target preference** (iteration 29 fix): At segment
          //     boundaries the click raycast often hits a neighboring seg=0
          //     face while the hover highlights the intended manual segment.
          //     Prefer `hoveredSegment` from store (what the user SEES as
          //     yellow highlight) over the clicked face's own label. Only
          //     fall back to clicked-face label when no hover exists.
          //
          // The old guard (`segments.some(s => s.id === label)`) was always true
          // because `auto_segment` emits a label-0 partition covering everything
          // left over, so one click flooded the ENTIRE model (REFUTE M1). The
          // reliable discriminator is the clicked partition's FACE SHARE, which
          // maps directly onto the complaint "因为它是一个大分区".
          const md = useAppStore.getState().meshData;
          const hovered = useAppStore.getState().hoveredSegment;
          const label = (hovered != null) ? hovered : (md?.segmentLabels?.[faceId] ?? undefined);
          const segs = md?.segments ?? [];
          const seg = label !== undefined ? segs.find((s) => s.id === label) : undefined;
          const total = md?.faceCount ?? 0;

          // `faceCount` comes from the backend DTO; fall back to an O(F) count
          // only when it is missing/zero so a stale DTO cannot mis-route.
          let segFaces = seg?.faceCount ?? 0;
          if (seg && segFaces <= 0 && md) {
            segFaces = 0;
            for (let i = 0; i < md.segmentLabels.length; i++) {
              if (md.segmentLabels[i] === seg.id) segFaces++;
            }
          }
          const share = seg && total > 0 ? segFaces / total : 1;
          const realSegs = segs.filter((s) => (s.faceCount ?? 0) > 0);
          const isManualRegion = (label ?? 0) >= MANUAL_SEGMENT_OFFSET;

          const fillWholeSegment =
            !opts?.wholeRegion && !!seg && (
              isManualRegion
                ? share <= MANUAL_REGION_MAX_SHARE  // allow large intentional regions
                : (realSegs.length > 1 && share <= WHOLE_SEGMENT_MAX_SHARE)
            );

          log.info("usePaintTool", "fill routing", {
            faceId, label, labelSrc: (hovered != null) ? "hover" : "click",
            segFaces, total,
            share: +share.toFixed(3),
            realSegs: realSegs.length,
            isManualRegion,
            mode: opts?.wholeRegion ? "whole-region" : fillWholeSegment ? "segment" : "local",
          });

          if (fillWholeSegment) {
            return fillSegment(faceId);
          }
          // radius 0 → explicit whole-connected-region flood (Shift+click), the
          // deliberate escape hatch kept for M3; otherwise a bounded local blob.
          result = await invoke<PaintResult>("fill_paint", {
            faceId,
            color: currentColor,
            radius: opts?.wholeRegion ? 0 : brushRadius,
          });
          break;
        }

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
