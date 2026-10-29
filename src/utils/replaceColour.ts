// ── Issue #29: colour find & replace — pure planning logic ─────────
// ONE semantic, per the owner: one click replaces EVERY region that used
// the source colour. The dry-run count ("将把 N 面 · M 个区域 从 #X 改为
// #Y") and the guard decisions run over the FRONTEND faceColors mirror
// (meshData.faceColors, flat RGBA bytes per face), so the preview updates
// instantly while the user picks — no IPC round trip. The apply itself is
// backend-authoritative (`replace_face_color` walks the real face_colors
// in one O(F) pass and records exactly one history entry); the numbers
// here are advisory.

/** Keep in sync with `DEFAULT_FACE_COLOR` in src-tauri/src/mesh/model.rs —
 * the never-painted / eraser grey. Alpha 255: paint always writes opaque. */
export const DEFAULT_FACE_COLOR_RGBA: [number, number, number, number] = [
  138, 138, 138, 255,
];

export interface ReplacePlan {
  faces: number;
  regions: number;
}

/**
 * Count the faces carrying `from` (exact RGBA match — alpha is part of the
 * colour identity, same rule as the used-colours list of #23) and how many
 * DISTINCT regions they span.
 */
export function planReplaceColour(
  faceColors: number[],
  segmentLabels: number[],
  from: [number, number, number, number]
): ReplacePlan {
  let faces = 0;
  const regions = new Set<number>();
  for (let f = 0, o = 0; o + 3 < faceColors.length; f++, o += 4) {
    if (
      faceColors[o] !== from[0] ||
      faceColors[o + 1] !== from[1] ||
      faceColors[o + 2] !== from[2] ||
      faceColors[o + 3] !== from[3]
    ) {
      continue;
    }
    faces++;
    regions.add(segmentLabels[f]);
  }
  return { faces, regions: regions.size };
}

/** Why the apply button is (or is not) disabled. */
export type ReplaceGuard = "same" | "defaultColour" | null;

/**
 * The issue's guard rails, in check order:
 * - `from == to` is a meaningless no-op → "same";
 * - replacing FROM the unpainted default grey would silently repaint every
 *   untouched face of the model → refused outright ("defaultColour") — the
 *   user can always achieve that deliberately with paint/fill; replacing TO
 *   the default grey is fine (it is exactly what the eraser writes).
 */
export function replaceGuard(
  from: [number, number, number, number],
  to: [number, number, number, number]
): ReplaceGuard {
  if (
    from[0] === to[0] &&
    from[1] === to[1] &&
    from[2] === to[2] &&
    from[3] === to[3]
  ) {
    return "same";
  }
  if (
    from[0] === DEFAULT_FACE_COLOR_RGBA[0] &&
    from[1] === DEFAULT_FACE_COLOR_RGBA[1] &&
    from[2] === DEFAULT_FACE_COLOR_RGBA[2] &&
    from[3] === DEFAULT_FACE_COLOR_RGBA[3]
  ) {
    return "defaultColour";
  }
  return null;
}
