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
    activeTool: "seed",
    seedPoints: [],
    suggestedSeeds: [],
    statusMessage: "",
    // Pin UI language: the app default now follows the OS locale, but these
    // tests assert Chinese strings regardless of the environment locale.
    language: "zh",
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

describe("SeedPanel.onResetAll — iteration 65「无法清空」修复", () => {
  it("点「清空分区」 → 调 reset_segmentation + 清掉所有 transient 数据", async () => {
    const user = userEvent.setup();
    // 预先塞入「旧的 × 按钮清不掉」的状态
    useAppStore.setState({
      seedPoints: [{ x: 1, y: 1, z: 1, faceIndex: 10 }],
      suggestedSeeds: [{ x: 2, y: 2, z: 2, faceIndex: 20 }],
      planarRegions: [{ plane: [0, 0, 0, 0], faceCount: 1, seed: { x: 0, y: 0, z: 0, faceIndex: 0 }, boundaryEdges: [], faceIndices: [] }],
      multiviewRegions: [],
      crossSectionRegions: [],
      seedEraseMode: true,
    });
    // 自动 confirm
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<SeedPanel />);
    await user.click(screen.getByRole("button", { name: /清空分区/ }));

    await waitFor(() =>
      expect(calls.some((c) => c.cmd === "reset_segmentation")).toBe(true)
    );
    expect(confirmSpy).toHaveBeenCalled();

    // 前端 transient 数据全部清掉:BoundaryLines / 种子 / 擦除模式 都不会残留
    const s = useAppStore.getState();
    expect(s.seedPoints).toHaveLength(0);
    expect(s.suggestedSeeds).toHaveLength(0);
    expect(s.planarRegions).toHaveLength(0);
    expect(s.seedEraseMode).toBe(false);
    expect(s.statusMessage).toContain("已清空分区");

    confirmSpy.mockRestore();
  });

  it("取消 confirm → 不发任何命令", async () => {
    const user = userEvent.setup();
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);

    render(<SeedPanel />);
    await user.click(screen.getByRole("button", { name: /清空分区/ }));
    await new Promise((r) => setTimeout(r, 30));

    expect(calls.some((c) => c.cmd === "reset_segmentation")).toBe(false);
    confirmSpy.mockRestore();
  });
});

// ── Iteration 66: 「我没办法退出这个界面」修复 ─────────────────────
// SeedPanel 之前没有 × / Esc 关闭出口,用户反馈「无法退出」。
// 标题行右上角 × + Esc 都必须切回 View 工具(种子数据保留)。
describe("SeedPanel.onClose — iteration 66「无法退出」修复", () => {
  it("点标题行 × → activeTool 切回 View(种子/算法数据保留)", async () => {
    const user = userEvent.setup();
    useAppStore.setState({
      activeTool: "seed",
      seedPoints: [{ x: 1, y: 1, z: 1, faceIndex: 10 }],
      suggestedSeeds: [{ x: 2, y: 2, z: 2, faceIndex: 20 }],
    });

    render(<SeedPanel />);
    const closeBtn = screen.getByRole("button", { name: /关闭 Seed 面板/ });
    await user.click(closeBtn);

    // activeTool 切走 → SeedPanel 会被条件渲染自动卸载
    expect(useAppStore.getState().activeTool).toBe("view");
    // 重要:已放置的种子应该保留,避免「关掉再回来还要重新布置」的体验断裂
    expect(useAppStore.getState().seedPoints).toHaveLength(1);
    expect(useAppStore.getState().suggestedSeeds).toHaveLength(1);
    // 不应该触发任何 Tauri 命令,纯前端本地切换
    expect(calls).toHaveLength(0);
  });

  it("Esc → activeTool 切回 View", async () => {
    useAppStore.setState({ activeTool: "seed" });
    render(<SeedPanel />);

    // 派发原生 keydown,模拟键盘 Esc
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await new Promise((r) => setTimeout(r, 10));
    expect(useAppStore.getState().activeTool).toBe("view");
  });
});

describe("SeedPanel 折角滑杆 — 下限与默认值回归（godzilla fuse 探针）", () => {
  // 0° 下限是刻死的：后端 fuse 在 2° 才能把光滑雕塑体（哥斯拉 86.7% 巨区 →
  // 13%）分出头/四肢/尾巴，见 segment::godzilla_soft_fold_feasibility /
  // fuse_floor_e2e。滑杆若退回 min=5，雕塑类模型会重新退化成一个巨区。
  // 默认值 2026-10-04 起为 5°（先生产品决策，原 2°）：硬表面模型 5° 首切
  // 更合理；雕塑类用户按滑杆提示手动调回 2-3°——下限 0° 保证了这条路畅通。
  it("折角滑杆 min=0（0° 已探针验证可终止）且默认值 5°", () => {
    // 前序失败用例的 /生长/ 选择器会误点「手动（生长）」模式 Tab 并把
    // segmentMode 泄漏为 manual；本用例只关心 auto 模式下的滑杆，显式归位。
    useAppStore.setState({ segmentMode: "auto" });
    const { container } = render(<SeedPanel />);
    const ranges = Array.from(
      container.querySelectorAll('input[type="range"]')
    ) as HTMLInputElement[];
    const fold = ranges.find((r) => r.min === "0");
    expect(fold).toBeTruthy();
    expect(fold!.max).toBe("35");
    expect(fold!.value).toBe("5");
  });
});
