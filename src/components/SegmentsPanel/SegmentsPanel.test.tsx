import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SegmentsPanel } from "./SegmentsPanel";
import { useAppStore } from "../../store/appStore";

// ── Tauri `invoke` mock with call capture (same pattern as SeedPanel) ──
const { mockInvoke, calls } = vi.hoisted(() => {
  const calls: { cmd: string; args: unknown }[] = [];
  const mockInvoke = vi.fn(async (cmd: string, args: unknown) => {
    calls.push({ cmd, args });
    if (cmd === "reset_segmentation") {
      return { segments: [], segmentLabels: [], faceColors: [] };
    }
    return {};
  });
  return { mockInvoke, calls };
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: mockInvoke }));

const resetStore = () => {
  useAppStore.setState({
    language: "zh",
    isLoaded: true,
    segments: [
      { id: 0, name: "r0", color: [200, 30, 30, 255], faceCount: 5 },
      { id: 1, name: "r1", color: [30, 200, 30, 255], faceCount: 7 },
    ],
    selectedSegment: null,
    hiddenSegments: new Set<number>(),
    mergePicked: [],
    statusMessage: "",
  });
};

beforeEach(() => {
  calls.length = 0;
  resetStore();
});

// ── Issue #28: 重置分区 used `window.confirm` as its guard — in the real
// WebView2 that resolves async via tauri-plugin-dialog, so the truthy
// Promise bypassed the sync guard and the reset ran unasked. The flow must
// go through the in-app ResetConfirmDialog (phase contract in
// utils/resetGate); these tests deliberately do NOT mock window.confirm.
describe("SegmentsPanel.onResetSegmentation — issue #28 in-app confirm", () => {
  it("点「重置」 → 先弹应用内确认框，确认后才调 reset_segmentation", async () => {
    const user = userEvent.setup();
    render(<SegmentsPanel />);

    await user.click(screen.getByRole("button", { name: "重置" }));

    // The click itself must not wipe anything — the dialog gates it.
    expect(calls.some((c) => c.cmd === "reset_segmentation")).toBe(false);
    expect(screen.getByText("清空所有分区")).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "确认清空" }));
    await waitFor(() =>
      expect(calls.filter((c) => c.cmd === "reset_segmentation")).toHaveLength(1)
    );
    // Dialog closed once the reset settled.
    await waitFor(() =>
      expect(screen.queryByText("清空所有分区")).toBeNull()
    );
  });

  it("取消确认 → 不发任何命令，对话框关闭", async () => {
    const user = userEvent.setup();
    render(<SegmentsPanel />);

    await user.click(screen.getByRole("button", { name: "重置" }));
    expect(screen.getByText("清空所有分区")).toBeTruthy();

    await user.click(screen.getByRole("button", { name: "取消" }));
    await waitFor(() =>
      expect(screen.queryByText("清空所有分区")).toBeNull()
    );

    expect(calls.some((c) => c.cmd === "reset_segmentation")).toBe(false);
  });

  it("确认后后端报错 → 对话框仍关闭，分区列表不动（finally 收尾）", async () => {
    const user = userEvent.setup();
    const prevImpl = mockInvoke.getMockImplementation();
    mockInvoke.mockImplementation(async (cmd: string, args: unknown) => {
      calls.push({ cmd, args });
      if (cmd === "reset_segmentation") {
        throw new Error("backend exploded");
      }
      return {};
    });

    try {
      render(<SegmentsPanel />);
      await user.click(screen.getByRole("button", { name: "重置" }));
      await user.click(screen.getByRole("button", { name: "确认清空" }));

      // The settled transition fires in the finally branch regardless of
      // the rejection — the dialog must not get stuck open.
      await waitFor(() =>
        expect(screen.queryByText("清空所有分区")).toBeNull()
      );
      // Regions survive a failed reset.
      expect(useAppStore.getState().segments).toHaveLength(2);
    } finally {
      mockInvoke.mockImplementation(prevImpl);
    }
  });
});
