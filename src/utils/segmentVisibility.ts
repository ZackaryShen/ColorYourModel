/** Pure set operations behind the per-region visibility toggles (issue #24).
 *
 * Hiding is display-only: these sets never touch labels, colours, the undo
 * timeline, or exports — the viewport's shader discards hidden faces and the
 * pick filter skips them. Rules pinned by segmentVisibility.test.ts:
 * new regions start visible, a multi-selection toggles as a whole (net
 * toggle, not a per-row coin flip), isolating hides everything else, and a
 * label that no longer exists leaves the set.
 */

/** Net-toggle `labels` against the hidden set: partly-hidden selections hide
 *  completely, fully-hidden selections come back completely. */
export function toggleHiddenLabels(prev: Set<number>, labels: number[]): Set<number> {
  const next = new Set(prev);
  const allHidden = labels.every((l) => next.has(l));
  for (const l of labels) {
    if (allHidden) next.delete(l);
    else next.add(l);
  }
  return next;
}

/** Hide everything except `label` — the isolate companion action. */
export function isolateLabel(allLabels: number[], label: number): Set<number> {
  const next = new Set<number>();
  for (const l of allLabels) {
    if (l !== label) next.add(l);
  }
  return next;
}

/** Drop hidden labels that no longer exist (merge / reset / undo rewrote
 *  the partition). A stale entry would otherwise hide a future region that
 *  gets the same label reissued. */
export function pruneHiddenLabels(prev: Set<number>, liveLabels: Set<number>): Set<number> {
  const next = new Set<number>();
  for (const l of prev) {
    if (liveLabels.has(l)) next.add(l);
  }
  return next;
}

// ─── Raycast filtering ─────────────────────────────────────────────
/** Minimal shape the filter needs (structurally satisfied by
 *  THREE.Intersection). */
export interface RayHitLike {
  faceIndex?: number | null | undefined;
}

/** Drop hits whose face belongs to a hidden region, so hidden geometry
 *  cannot be picked, painted, hovered, or used as a fill target (issue
 *  #24). Hits without face info pass through unchanged. */
export function filterVisibleHits<T extends RayHitLike>(
  hits: T[],
  hidden: Set<number>,
  labels: number[] | undefined
): T[] {
  return hits.filter((h) => {
    if (h.faceIndex == null) return true;
    const label = labels?.[h.faceIndex];
    return label === undefined || !hidden.has(label);
  });
}

/** First hit whose face is not on a hidden region (raycast consumers want
 *  exactly one). Null when every hit is hidden. */
export function firstVisibleHit<T extends RayHitLike>(
  hits: T[],
  hidden: Set<number>,
  labels: number[] | undefined
): T | null {
  return filterVisibleHits(hits, hidden, labels)[0] ?? null;
}
