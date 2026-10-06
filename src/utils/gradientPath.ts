/**
 * Path-gradient brush math (0.2.0-P2, req #7) — pure functions, extracted
 * from Viewport for unit testing (same pattern as viewGizmoModel.ts).
 *
 * The stroke color interpolates colorA → colorB over a screen-space arc
 * length, normalized by a user-set fade length expressed as a percentage of
 * the viewport diagonal. Purely screen-space by design: no px↔mm conversion
 * and no ortho-zoom dependence in the UX.
 */

export interface GradientStrokeState {
  /** Screen-space arc length accumulated so far, px. */
  accumPx: number;
  prev: { x: number; y: number } | null;
}

/** "#rrggbb" → RGB tuple (alpha forced 255 downstream). */
export function hexToRgb(hex: string): [number, number, number] {
  const m = /^#?([0-9a-fA-F]{6})$/.exec(hex.trim());
  if (!m) return [138, 138, 138];
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

/** Advance the arc-length accumulator by one pointer sample. */
export function accumulateArc(
  state: GradientStrokeState,
  clientX: number,
  clientY: number
): GradientStrokeState {
  const d = state.prev
    ? Math.hypot(clientX - state.prev.x, clientY - state.prev.y)
    : 0;
  return { accumPx: state.accumPx + d, prev: { x: clientX, y: clientY } };
}

/** Per-sample gradient color at the accumulated arc length. */
export function pathGradientColor(
  accumPx: number,
  fadePx: number,
  colorA: [number, number, number],
  colorB: [number, number, number]
): [number, number, number, number] {
  const t = Math.min(1, accumPx / Math.max(1, fadePx));
  return [
    Math.round(colorA[0] + (colorB[0] - colorA[0]) * t),
    Math.round(colorA[1] + (colorB[1] - colorA[1]) * t),
    Math.round(colorA[2] + (colorB[2] - colorA[2]) * t),
    255,
  ];
}

/** Whether `tool` paints along a path with a per-sample gradient color. */
export function isGradientTool(tool: string): boolean {
  return tool === "gradient";
}
