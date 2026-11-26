import { PaintTool } from "../types/mesh";

export type GradientColor = [number, number, number, number];
export type GradientTool = PaintTool.Brush | PaintTool.Fill;

export const MIN_GRADIENT_STOPS = 2;
export const MAX_GRADIENT_STOPS = 8;

export const DEFAULT_GRADIENT_STOPS: GradientColor[] = [
  [235, 65, 75, 255],
  [252, 143, 45, 255],
  [250, 219, 72, 255],
  [83, 197, 105, 255],
  [53, 140, 245, 255],
  [149, 79, 230, 255],
];

export function isGradientTool(tool: PaintTool): tool is GradientTool {
  return tool === PaintTool.Brush || tool === PaintTool.Fill;
}
