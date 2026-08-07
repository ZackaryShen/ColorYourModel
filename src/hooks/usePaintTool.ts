import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../store/appStore";
import { PaintResult, PaintTool } from "../types/mesh";
import { log } from "../utils/logger";

/**
 * D2 (product ruling, 2026-08-06): a large partition (>80% share) is filled
 * WHOLE, never degraded. The share thresholds that used to demote such clicks
 * to a local brush-radius blob are deleted (docs/06 §2.1 item 3). What remains
 * here is only what OTHER consumers need:
 *   - `WHOLE_SEGMENT_MAX_SHARE`: still consulted by Viewport's giantSegmentIds
 *     to suppress hover highlight on background partitions (Gate 0d backlog —
 *     removing it would flood the highlight overlay; see docs/06 §2.2).
 *   - `MANUAL_SEGMENT_OFFSET`: label >= this is a manual region.
 */
export const WHOLE_SEGMENT_MAX_SHARE = 0.8;

/// Labels >= this value are manually-created regions (lasso / freehand).
/// These should ALWAYS be filled as whole partitions — the user explicitly
/// drew them and expects fill to cover the entire region (iteration 21 fix).
export const MANUAL_SEGMENT_OFFSET = 100_000;

/** How a Fill click was actually routed. The HUD needs this reported rather
 *  than re-derived: only the `segment` route claims to cover the highlighted
 *  partition exactly, so comparing "faces filled" against "faces highlighted"
 *  is meaningful for that route alone. Asserting equality on a local blob or a
 *  connected-region flood manufactures a mismatch that is in fact by design —
 *  part of what iterations 25-30 spent six rounds chasing. */
export type FillRouting = "segment" | "whole-region" | "local" | "n/a";

export type PaintOutcome = PaintResult & {
  fillRouting: FillRouting;
  /** Label the fill targeted, or null when the route is not label-based. */
  fillTarget: number | null;
};

export function usePaintTool() {
  const activeTool = useAppStore((s) => s.activeTool);
  const brushRadius = useAppStore((s) => s.brushRadius);
  const brushStrength = useAppStore((s) => s.brushStrength);
  const brushFalloff = useAppStore((s) => s.brushFalloff);
  const currentColor = useAppStore((s) => s.currentColor);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const setSelectedSegment = useAppStore((s) => s.setSelectedSegment);

  const fillSegment = async (
    faceId: number,
    overrideSegmentId?: number
  ): Promise<PaintOutcome | null> => {
    const meshData = useAppStore.getState().meshData;
    if (!meshData || !meshData.segmentLabels.length) return null;
    const segmentId = overrideSegmentId ?? meshData.segmentLabels[faceId];
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
      return { ...result, fillRouting: "segment", fillTarget: segmentId };
    } catch (e) {
      log.error("usePaintTool", "fillSegment failed", { segmentId, error: String(e) });
      setStatusMessage(`分区填充失败：${e}`);
      return null;
    }
  };

  const paintFace = async (
    faceId: number,
    opts?: { wholeRegion?: boolean; hoveredSegment?: number | null; strokeId?: number | null }
  ): Promise<PaintOutcome | null> => {
    log.debug("usePaintTool", `paintFace(${faceId})`, { tool: activeTool, color: currentColor });
    try {
      let result: PaintResult;
      let fillRouting: FillRouting = "n/a";
      let fillTarget: number | null = null;

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
          //     Prefer the caller-supplied hoveredSegment (what the user SEES
          //     as yellow highlight) over the clicked face's own label. Only
          //     fall back to clicked-face label when no hover exists.
          //
          //   • **D2 (product ruling, 2026-08-06)**: a large partition (>80 %
          //     share) is filled WHOLE, never degraded. The share thresholds
          //     and the `realSegs.length > 1` gate that used to demote such
          //     clicks to a local brush-radius blob were deleted (docs/06 §2.1
          //     item 3). The old guard made "the whole model is one partition"
          //     indistinguishable from "I clicked the background": hover
          //     highlighted the partition but the fill only painted a blob of
          //     `brushRadius`, which read as "Fill uses the brush diameter".
          //     `!!seg` is kept deliberately — with an EMPTY segments list
          //     (autoSegment failed or was skipped) there is no partition to
          //     fill, and routing to fillSegment would flood the entire model
          //     through label 0 (REFUTE, autoSegment-failure path).
          const md = useAppStore.getState().meshData;
          const hovered = opts?.hoveredSegment ?? null;
          const label = (hovered != null) ? hovered : (md?.segmentLabels?.[faceId] ?? undefined);
          const segs = md?.segments ?? [];
          const seg = label !== undefined ? segs.find((s) => s.id === label) : undefined;

          // D2: any existing partition is fillable whole, regardless of share.
          const fillWholeSegment = !opts?.wholeRegion && !!seg && label !== undefined;

          log.info("usePaintTool", "fill routing", {
            faceId, label, labelSrc: (hovered != null) ? "hover" : "click",
            mode: opts?.wholeRegion ? "whole-region" : fillWholeSegment ? "segment" : "local",
          });

          if (fillWholeSegment) {
            return fillSegment(faceId, label);
          }
          fillRouting = opts?.wholeRegion ? "whole-region" : "local";
          fillTarget = label ?? null;
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
            strokeId: opts?.strokeId ?? null,
          });
          break;

        case PaintTool.Spray:
          result = await invoke<PaintResult>("spray_paint", {
            centerFace: faceId,
            radius: brushRadius,
            strength: brushStrength,
            color: currentColor,
            density: 50,
            strokeId: opts?.strokeId ?? null,
          });
          break;

        case PaintTool.SmartBrush:
          result = await invoke<PaintResult>("smart_brush_paint", {
            centerFace: faceId,
            radius: brushRadius,
            strength: brushStrength,
            falloffMode: brushFalloff,
            color: currentColor,
            strokeId: opts?.strokeId ?? null,
          });
          break;

        case PaintTool.Eraser:
          result = await invoke<PaintResult>("erase_paint", {
            centerFace: faceId,
            radius: brushRadius,
            strokeId: opts?.strokeId ?? null,
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
      return { ...result, fillRouting, fillTarget };
    } catch (e) {
      log.error("usePaintTool", "Paint command failed", { faceId, tool: activeTool, error: String(e) });
      setStatusMessage(`Paint error: ${e}`);
      return null;
    }
  };

  return { paintFace, fillSegment };
}
