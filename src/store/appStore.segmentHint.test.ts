import { beforeEach, describe, expect, it } from "vitest";
import { useAppStore } from "./appStore";

/**
 * Post-fuse segment-view suggestion lifecycle (Viewport top-centre card).
 * Semantics under test: raised only from paint view, cleared by honoring
 * (switching the segment view on) or dismissing, and never resurrected by
 * toggling the view back off.
 */
beforeEach(() => {
  useAppStore.setState({ segmentView: false, segmentViewHint: false });
});

describe("segmentViewHint lifecycle", () => {
  it("raises the hint after a fuse while in paint view", () => {
    useAppStore.getState().requestSegmentViewHint();
    expect(useAppStore.getState().segmentViewHint).toBe(true);
  });

  it("refuses to raise the hint when the segment view is already on", () => {
    useAppStore.setState({ segmentView: true });
    useAppStore.getState().requestSegmentViewHint();
    expect(useAppStore.getState().segmentViewHint).toBe(false);
  });

  it("honoring the suggestion (switching on) clears the hint", () => {
    useAppStore.setState({ segmentViewHint: true });
    useAppStore.getState().setSegmentView(true);
    expect(useAppStore.getState().segmentViewHint).toBe(false);
  });

  it("toggling the view off does not resurrect a dismissed hint", () => {
    useAppStore.getState().requestSegmentViewHint();
    useAppStore.getState().dismissSegmentViewHint();
    useAppStore.getState().setSegmentView(true);
    useAppStore.getState().setSegmentView(false);
    expect(useAppStore.getState().segmentViewHint).toBe(false);
  });

  it("dismiss keeps it down", () => {
    useAppStore.getState().requestSegmentViewHint();
    useAppStore.getState().dismissSegmentViewHint();
    expect(useAppStore.getState().segmentViewHint).toBe(false);
  });
});
