import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SeedPanel } from "./SeedPanel";
import { useAppStore } from "../store/appStore";
import type { SeedPoint } from "../types/mesh";

// ── Tauri `invoke` mock with call capture ──────────────────────────
// `vi.hoisted` lets the vi.mock factory reference the same mock instance.
const { mockInvoke, calls } = vi.hoisted(() => {
  const calls: { cmd: string; args: unknown }[] = [];
  const mockInvoke = vi.fn(async (cmd: string, args: unknown) => {
    calls.push({ cmd, args });
    if (cmd === "recommend_seeds") {
      // 3 advisory seeds, same shape the backend returns.
      return [
        { point: [1, 2, 3], faceIndex: 10 },
        { point: [4, 5, 6], faceIndex: 20 },
        { point: [7, 8, 9], faceIndex: 30 },
      ];
    }
    if (cmd === "seed_grow") {
      return {
        segments: [{ label: 0, name: "r0", color: "#ffffff" }],
        segmentLabels: [0, 0, 1],
        faceColors: [],
      };
    }
    return {};
  });
  return { mockInvoke, calls };
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: mockInvoke }));

const resetStore = () => {
  useAppStore.setState({
    activeTool: "seed",
    seedPoints: [],
    suggestedSeeds: [],
    statusMessage: "",
  } as Partial<ReturnType<typeof useAppStore.getState>>);
};

beforeEach(() => {
  calls.length = 0;
  resetStore();
});

const suggestBtn = () => screen.getByRole("button", { name: /推荐种子/ });
const growBtn = () => screen.getByRole("button", { name: /生长/ });

describe("SeedPanel.onGrow — iter60/61/63 回归（建议种子 → 生成 一步到位）", () => {
  it("只有 suggestedSeeds（无 accepted seedPoints）也能生长", async () => {
    const user = userEvent.setup();
    render(<SeedPanel />);

    // 点「建议种子」→ 拿到 3 个建议
    await user.click(suggestBtn());
    await waitFor(() =>
      expect(useAppStore.getState().suggestedSeeds.length).toBe(3)
    );
    expect(calls.some((c) => c.cmd === "recommend_seeds")).toBe(true);

    // 直接点「生成」—— 这是用户之前卡住的操作
    await user.click(growBtn());
    await waitFor(() =>
      expect(calls.some((c) => c.cmd === "seed_grow")).toBe(true)
    );

    const grow = calls.find((c) => c.cmd === "seed_grow")!;
    const payload = (grow.args as { seeds: { point: number[]; face_index: number }[] })
      .seeds;
    // 回归断言：必须拿 3 个建议种子去生长（旧代码会静默 return）
    expect(payload).toHaveLength(3);
    expect(payload[0]).toEqual({ point: [1, 2, 3], face_index: 10 });
    // iter61：生长后保留建议幽灵标记作为微调参考，不清空
    expect(useAppStore.getState().suggestedSeeds.length).toBe(3);
  });

  it("手动种子与建议种子共同参与生长（并集）", async () => {
    const user = userEvent.setup();
    const accepted: SeedPoint[] = [
      { x: 0, y: 0, z: 0, faceIndex: 1 },
      { x: 1, y: 1, z: 1, faceIndex: 2 },
    ];
    useAppStore.setState({
      seedPoints: accepted,
      suggestedSeeds: [{ x: 9, y: 9, z: 9, faceIndex: 99 }],
    });
    render(<SeedPanel />);

    await user.click(growBtn());
    await waitFor(() =>
      expect(calls.some((c) => c.cmd === "seed_grow")).toBe(true)
    );
    const grow = calls.find((c) => c.cmd === "seed_grow")!;
    const payload = (grow.args as { seeds: { point: number[]; face_index: number }[] })
      .seeds;
    // iter63：并集 = 2 手动 + 1 建议 = 3，而非只取手动种子
    expect(payload).toHaveLength(3);
    expect(payload[0]).toEqual({ point: [0, 0, 0], face_index: 1 });
    expect(payload[2]).toEqual({ point: [9, 9, 9], face_index: 99 });
  });

  it("什么种子都没有 → 不调 seed_grow，给出提示", async () => {
    const user = userEvent.setup();
    render(<SeedPanel />);

    await user.click(growBtn());
    // 让微任务跑完
    await new Promise((r) => setTimeout(r, 30));
    expect(calls.some((c) => c.cmd === "seed_grow")).toBe(false);
    expect(useAppStore.getState().statusMessage).toContain("请至少放置一个种子");
  });
});
