import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SegmentsPanel } from "./SegmentsPanel/SegmentsPanel";
import { useAppStore } from "../store/appStore";

// ── Tauri `invoke` mock with call capture (same approach as SeedPanel.test) ──
const { mockInvoke, calls } = vi.hoisted(() => {
  const calls: { cmd: string; args: unknown }[] = [];
  const mockInvoke = vi.fn(async (cmd: string, args: unknown) => {
    calls.push({ cmd, args });
    if (cmd === "merge_segments") {
      // r2 (id 2) absorbed into r1 (id 1): one region left in the mocked result.
      return {
        segments: [{ id: 1, name: "r1", color: null, faceCount: 30 }],
        segmentLabels: [1, 1, 1],
        movedFaces: 20,
      };
    }
    return {};
  });
  return { mockInvoke, calls };
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: mockInvoke }));

const resetStore = () => {
  useAppStore.setState({
    segments: [
      { id: 1, name: "r1", color: null, faceCount: 10 },
      { id: 2, name: "r2", color: null, faceCount: 20 },
      { id: 3, name: "r3", color: null, faceCount: 30 },
    ],
    selectedSegment: null,
    mergePicked: [],
    isLoaded: true,
    statusMessage: "",
    // Pin UI language: these tests assert Chinese strings regardless of the
    // environment locale (same precedent as SeedPanel.test).
    language: "zh",
  } as Partial<ReturnType<typeof useAppStore.getState>>);
};

beforeEach(() => {
  calls.length = 0;
  resetStore();
});

// user-event v14 does not forward per-call modifier options on click(); the
// reliable way to simulate Ctrl+click is holding Control via the keyboard
// state around the click (verified by probe).
const ctrlClick = async (
  user: ReturnType<typeof userEvent.setup>,
  el: HTMLElement
) => {
  await user.keyboard("{Control>}");
  await user.click(el);
  await user.keyboard("{/Control}");
};

// Issue #16: panel rows must drive the SAME store-backed merge multi-select as
// the viewport's Ctrl+click, and merging goes through the shared hook with
// (anchor, rest) argument order.
describe("SegmentsPanel merge multi-select (store-backed picked)", () => {
  it("ctrl+click two rows → merge button names the anchor → merge_segments(anchor, [rest]) → picked collapses to [anchor]", async () => {
    const user = userEvent.setup();
    render(<SegmentsPanel />);

    // No merge button while fewer than two rows are picked.
    expect(screen.queryByRole("button", { name: /合并/ })).toBeNull();

    await ctrlClick(user, screen.getByText("r1"));
    await ctrlClick(user, screen.getByText("r2"));

    // Ordered store selection with first-pick-is-anchor; highlight follows it.
    expect(useAppStore.getState().mergePicked).toEqual([1, 2]);
    expect(useAppStore.getState().selectedSegment).toBe(1);

    // Button text names the anchor, not just the count.
    const mergeBtn = screen.getByRole("button", { name: "合并 2 个分区到「r1」" });
    await user.click(mergeBtn);

    await waitFor(() =>
      expect(calls.some((c) => c.cmd === "merge_segments")).toBe(true)
    );
    const merge = calls.find((c) => c.cmd === "merge_segments")!;
    expect(merge.args).toEqual({ targetId: 1, sourceIds: [2] });

    // After success the selection collapses to the anchor so the user can keep
    // merging into it, and the status bar reports the moved face count.
    await waitFor(() =>
      expect(useAppStore.getState().mergePicked).toEqual([1])
    );
    expect(useAppStore.getState().selectedSegment).toBe(1);
    expect(useAppStore.getState().statusMessage).toContain("已合并 2 个分区（20 个面）");
  });

  it("plain click re-anchors the selection to one row; clicking it again deselects", async () => {
    const user = userEvent.setup();
    render(<SegmentsPanel />);

    await ctrlClick(user, screen.getByText("r1"));
    await ctrlClick(user, screen.getByText("r2"));
    expect(useAppStore.getState().mergePicked).toEqual([1, 2]);

    // Plain click discards the multi-select and re-anchors to r3.
    await user.click(screen.getByText("r3"));
    expect(useAppStore.getState().mergePicked).toEqual([3]);
    expect(useAppStore.getState().selectedSegment).toBe(3);

    // Clicking the only picked row again clears the merge selection.
    await user.click(screen.getByText("r3"));
    expect(useAppStore.getState().mergePicked).toEqual([]);
  });
});
