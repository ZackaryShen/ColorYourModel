/** Ctrl+click region-merge gesture state machine (issue #16).
 *
 * Auto-seeding splits the model into more regions than the user wants; this
 * gesture lets them absorb regions into one without re-seeding or visiting
 * the Segments panel: Ctrl+click picks an anchor region, further Ctrl+clicks
 * absorb other regions into it (the anchor stays, so a chain of clicks joins
 * any number of regions), clicking the anchor again or pressing Esc cancels.
 *
 * Pure: the viewport translates pointer events into clicks and effects into
 * highlight/status/IPC. The merge itself rides the SAME `merge_segments`
 * pipeline the Segments panel drives — there is no second merge path.
 * Tested in mergeGesture.test.ts.
 */

export interface MergeGestureState {
  /** The surviving region's label, or null when no gesture is in progress. */
  anchor: number | null;
}

export type MergeGestureEffect =
  | { type: "anchor"; label: number }
  | { type: "merge"; target: number; source: number }
  | { type: "clear" }
  | { type: "blocked"; reason: "single-region" };

/** Feed one Ctrl+click (on-model, live label) through the gesture. */
export function mergeGestureClick(
  state: MergeGestureState,
  clicked: number,
  liveRegionIds: number[]
): { state: MergeGestureState; effect: MergeGestureEffect } {
  if (liveRegionIds.length < 2) {
    return { state, effect: { type: "blocked", reason: "single-region" } };
  }
  if (state.anchor === null) {
    return { state: { anchor: clicked }, effect: { type: "anchor", label: clicked } };
  }
  if (clicked === state.anchor) {
    return { state: { anchor: null }, effect: { type: "clear" } };
  }
  return { state, effect: { type: "merge", target: state.anchor, source: clicked } };
}

/** Esc / model switch: drop the anchor. */
export function mergeGestureCancel(
  state: MergeGestureState
): { state: MergeGestureState; effect: MergeGestureEffect } {
  return { state: { anchor: null }, effect: { type: "clear" } };
}
