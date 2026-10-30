import { PaintTool } from "../types/mesh";

export type ControlsHintKey =
  | "controls.viewHint"
  | "controls.brushHint"
  | "controls.editHint"
  | "controls.fillHint";

/**
 * The ONE source for the per-tool mouse-binding hint text.
 *
 * Before the 2026-10-02 GUI audit (B4) this mapping lived privately inside
 * Viewport's ControlsHelp while StatusBar rendered a static `status.tip`
 * string ("Left: Rotate (View) | …") that contradicted every non-view tool.
 * Both consumers now resolve through this function, so the two surfaces can
 * never disagree again.
 */
export function hintKeyForTool(tool: PaintTool): ControlsHintKey {
  if (tool === PaintTool.View) return "controls.viewHint";
  // Issue #16 (field feedback #4) → #29 unification: fill is the Ctrl+click
  // multi-select's home tool alongside View, so its hint advertises
  // Ctrl+click — and warns that a plain click means fill.
  if (tool === PaintTool.Fill) return "controls.fillHint";
  const isBrush =
    tool === PaintTool.Brush ||
    tool === PaintTool.Spray ||
    tool === PaintTool.SmartBrush ||
    tool === PaintTool.Eraser;
  if (isBrush) return "controls.brushHint";
  return "controls.editHint";
}
