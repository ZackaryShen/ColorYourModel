import { describe, it, expect } from "vitest";
import {
  mergeGestureClick,
  mergeGestureCancel,
  type MergeGestureState,
} from "./mergeGesture";

// Issue #16: Ctrl+click a seeded area, then Ctrl+click others, to absorb them
// into one region — the viewport front-end of the SAME merge pipeline the
// Segments panel drives. The state machine below is the whole gesture logic;
// the viewport only translates clicks into `mergeGestureClick` calls and
// effects into highlight/status/IPC.

const REGIONS = [7, 12, 30];

describe("mergeGestureClick", () => {
  it("sets the anchor on the first click without merging", () => {
    const { state, effect } = mergeGestureClick({ anchor: null }, 12, REGIONS);
    expect(state).toEqual({ anchor: 12 });
    expect(effect).toEqual({ type: "anchor", label: 12 });
  });

  it("absorbs the next different region into the anchor, anchor retained for chaining", () => {
    const { state, effect } = mergeGestureClick({ anchor: 7 }, 12, REGIONS);
    expect(effect).toEqual({ type: "merge", target: 7, source: 12 });
    // The anchor survives the backend merge (merge_segments keeps the target
    // id), so a third Ctrl+click chains another absorption without re-picking.
    expect(state).toEqual({ anchor: 7 });
  });

  it("clears when the anchor region is clicked again", () => {
    const { state, effect } = mergeGestureClick({ anchor: 7 }, 7, REGIONS);
    expect(state).toEqual({ anchor: null });
    expect(effect).toEqual({ type: "clear" });
  });

  it("is blocked on a single-region model, state untouched", () => {
    const { state, effect } = mergeGestureClick({ anchor: null }, 7, [7]);
    expect(effect).toEqual({ type: "blocked", reason: "single-region" });
    expect(state).toEqual({ anchor: null });
    // Even a stale anchor must not merge when nothing else exists.
    const again = mergeGestureClick({ anchor: 7 }, 7, [7]);
    expect(again.effect).toEqual({ type: "blocked", reason: "single-region" });
    expect(again.state).toEqual({ anchor: 7 });
  });

  it("anchor survives a chained absorption of several regions in a row", () => {
    let g: MergeGestureState = { anchor: null };
    g = mergeGestureClick(g, 7, REGIONS).state;
    g = mergeGestureClick(g, 12, REGIONS).state;
    const last = mergeGestureClick(g, 30, REGIONS);
    expect(last.effect).toEqual({ type: "merge", target: 7, source: 30 });
  });
});

describe("mergeGestureCancel", () => {
  it("drops the anchor", () => {
    const { state, effect } = mergeGestureCancel({ anchor: 7 });
    expect(state).toEqual({ anchor: null });
    expect(effect).toEqual({ type: "clear" });
  });

  it("is a no-op without an anchor", () => {
    const { state, effect } = mergeGestureCancel({ anchor: null });
    expect(state).toEqual({ anchor: null });
    expect(effect).toEqual({ type: "clear" });
  });
});
