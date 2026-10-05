import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../store/appStore";
import { useT } from "../i18n";
import { PaintResult, PaintTool } from "../types/mesh";
import { log } from "../utils/logger";

/**
 * D2 (product ruling, 2026-08-06): a large partition (>80% share) is filled
 * WHOLE, never degraded. The share thresholds that used to demote such clicks
 * to a local brush-radius blob are deleted (docs/06 §2.1 item 3). Segment
 * highlight is now shader-based (option B): a per-vertex `aSegLabel` attribute
 * plus a `uHighlightLabel` uniform tint the hovered/selected partition on the
 * GPU in O(1), so the old `giantSegmentIds` performance guard is gone — even a
 * whole-model partition highlights instantly instead of rebuilding an overlay
 * geometry (docs/06 §2.2).
 */
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
  const t = useT();
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
      setStatusMessage(t("fill.done", segmentId, result.updatedFaces.length, hex));
      return { ...result, fillRouting: "segment", fillTarget: segmentId };
    } catch (e) {
      log.error("usePaintTool", "fillSegment failed", { segmentId, error: String(e) });
      setStatusMessage(t("fill.failed", String(e)));
      return null;
    }
  };

  const paintFace = async (
    faceId: number,
    opts?: {
      wholeRegion?: boolean;
      hoveredSegment?: number | null;
      strokeId?: number | null;
      /** 0.2.0-P2 gradient (path mode): the exact color this sample was
       *  computed to carry, overriding the uniform currentColor. */
      colorOverride?: [number, number, number, number];
      /** 0.2.0-P2 gradient (radial mode): paint a graded disc around faceId. */
      gradientRadial?: boolean;
    }
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
          //   • **Region authority (product ruling, 2026-08-27)**: the fill
          //     target is ALWAYS the partition containing the CLICKED face —
          //   `meshData.segmentLabels[faceId]`. The iter29 "hover preference"
          //     and the iter29 v5 stale-cache fallback are retired: routing a
          //     click through a hover snapshot (let alone one cached from an
          //     earlier pointer position) is what let "fill C red" repaint a
          //     different region entirely — the reported "colours repel each
          //     other" defect. Region info is the single source of truth;
          //     the shader highlight follows it because pointermove highlights
          //     the region under the cursor through the same labels array.
          const md = useAppStore.getState().meshData;
          const label = md?.segmentLabels?.[faceId] ?? undefined;
          const segs = md?.segments ?? [];
          // `!!seg` is deliberate: with an EMPTY segments list (autoSegment
          // failed or was skipped) there is no partition to fill, and routing
          // to fillSegment would flood the entire model through label 0.
          const seg = label !== undefined ? segs.find((s) => s.id === label) : undefined;

          // D2: any existing partition is fillable whole, regardless of share.
          const fillWholeSegment = !opts?.wholeRegion && !!seg && label !== undefined;

          log.info("usePaintTool", "fill routing", {
            faceId, label, labelSrc: "click",
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
            color: opts?.colorOverride ?? currentColor,
            strokeId: opts?.strokeId ?? null,
          });
          break;

        case PaintTool.Gradient: {
          // 0.2.0-P2 req #7. Path mode reuses brush_paint: the VIEWPORT
          // computes each sample's color along the stroke (arc-length t) and
          // ships it as colorOverride — one undo per stroke via strokeId.
          // Radial mode defers to the backend selector, which grades the disc
          // per-face (t = distance/radius; see paint/gradient.rs).
          if (opts?.gradientRadial) {
            const g = useAppStore.getState();
            result = await invoke<PaintResult>("gradient_radial_paint", {
              centerFace: faceId,
              radius: brushRadius,
              colorInner: hexToRgba(g.gradientColorA),
              colorOuter: hexToRgba(g.gradientColorB),
              strokeId: opts?.strokeId ?? null,
            });
            break;
          }
          result = await invoke<PaintResult>("brush_paint", {
            centerFace: faceId,
            radius: brushRadius,
            strength: brushStrength,
            falloffMode: brushFalloff,
            color: opts?.colorOverride ?? currentColor,
            strokeId: opts?.strokeId ?? null,
          });
          break;
        }

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
          setStatusMessage(t("paint.pickedColor", color[0], color[1], color[2]));
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
      setStatusMessage(t("paint.error", String(e)));
      return null;
    }
  };

  return { paintFace, fillSegment };
}


/** "#rrggbb" → RGBA tuple (alpha 255). Local to the gradient tool; the color
 *  panel palette works in tuples, the <input type=color> options work in hex. */
export function hexToRgba(hex: string): [number, number, number, number] {
  const m = /^#?([0-9a-fA-F]{6})$/.exec(hex.trim());
  if (!m) return [138, 138, 138, 255];
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255, 255];
}
