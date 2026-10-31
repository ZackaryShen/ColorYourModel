import type { UsedColorEntry } from "../types/mesh";

/**
 * UI-side condensing of the backend used-colour histogram (acceptance round
 * 2, feedback #4/#5).
 *
 * The brush blends colours per face (strength × falloff), so a single drag
 * can put hundreds of near-identical RGBA variants into `face_colors`. The
 * backend histogram counts DISTINCT colours (#23 contract: two similar
 * colours must stay two colours — replace relies on exact matching), which
 * made the panel explode into noise swatches.
 *
 * The panel's job is picking, not exactness, so we aggregate here:
 * - default grey is dropped entirely (feedback #5): "never painted" is not a
 *   used colour; the replace section's guards still cover the current-colour
 *   path;
 * - entries bucket by RGB >> 5 (8 levels per channel ≈ imperceptible deltas)
 *   and each bucket shows the MOST-PAINTED exact colour as its face — click
 *   still sets a colour that verifiably exists on the model;
 * - faces sum across the bucket; regions use the MAX (overlapping labels
 *   across variants would double-count a sum).
 */
export function condenseUsedColours(entries: UsedColorEntry[]): UsedColorEntry[] {
  type Bucket = {
    faces: number;
    regions: number;
    best: UsedColorEntry;
  };
  const buckets = new Map<string, Bucket>();
  for (const e of entries) {
    // Feedback #5: the never-painted default grey is not a "used colour".
    if (e.isDefault) continue;
    const key = `${e.color[0] >> 5},${e.color[1] >> 5},${e.color[2] >> 5}`;
    const b = buckets.get(key);
    if (!b) {
      buckets.set(key, { faces: e.faces, regions: e.regions, best: e });
      continue;
    }
    b.faces += e.faces;
    b.regions = Math.max(b.regions, e.regions);
    if (e.faces > b.best.faces) b.best = e;
  }
  const out = [...buckets.values()].map((b) => ({
    color: b.best.color,
    faces: b.faces,
    regions: b.regions,
    isDefault: false,
  }));
  out.sort((x, y) => y.faces - x.faces || String(x.color).localeCompare(String(y.color)));
  return out;
}
