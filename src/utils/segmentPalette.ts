/**
 * The palette that gives a segment its identity colour.
 *
 * This existed twice — as hex strings in `SegmentsPanel` and as RGB triples in
 * `useMesh`, with a comment on the second copy asking the reader to keep them
 * in sync by hand. They had already drifted in a way that matters more than the
 * values: the panel indexed the palette by the segment's *position in the list*
 * while the viewport indexed it by the segment's *label*, so the dot next to a
 * region was a different colour from the region itself as soon as any label was
 * missing from the 0..N run (which is now the normal case — `preserve_manual`
 * deliberately leaves holes, and manual labels start at 100_000).
 *
 * Index by label, never by list position. `segmentColorHex(seg.id)` and
 * `segmentColorRgb(label)` are the only supported accessors for that reason.
 */
const SEGMENT_COLORS_HEX = [
  "#e74c3c", "#3498db", "#2ecc71", "#f1c40f", "#9b59b6",
  "#e67e22", "#1abc9c", "#e91e63", "#00bcd4", "#8bc34a",
  "#ff9800", "#795548", "#607d8b", "#ff5722", "#673ab7",
] as const;

function hexToRgb(hex: string): [number, number, number] {
  const n = parseInt(hex.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

/**
 * RGB form of the same palette, derived so the two cannot disagree.
 *
 * Exported for the vertex-colour loop in `useMesh`, which runs once per face on
 * a mesh that can hold 1.5M of them and therefore indexes the array directly
 * rather than paying for an accessor call.
 */
export const SEGMENT_COLORS_RGB: [number, number, number][] =
  SEGMENT_COLORS_HEX.map(hexToRgb);

export const SEGMENT_PALETTE_SIZE = SEGMENT_COLORS_HEX.length;

/** Identity colour for a segment label, as a CSS hex string. */
export function segmentColorHex(label: number): string {
  return SEGMENT_COLORS_HEX[label % SEGMENT_PALETTE_SIZE];
}

/** Identity colour for a segment label, as an 0-255 RGB triple. */
export function segmentColorRgb(label: number): [number, number, number] {
  return SEGMENT_COLORS_RGB[label % SEGMENT_PALETTE_SIZE];
}
