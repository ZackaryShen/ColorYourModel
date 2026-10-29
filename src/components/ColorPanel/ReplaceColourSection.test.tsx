import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ReplaceColourSection } from "./ReplaceColourSection";
import { useAppStore } from "../../store/appStore";
import type { MeshData } from "../../types/mesh";

// ── Tauri `invoke` mock: capture the replace call, return a shaped result ──
const { mockInvoke, calls } = vi.hoisted(() => {
  const calls: { cmd: string; args: unknown }[] = [];
  const mockInvoke = vi.fn(async (cmd: string, args: unknown) => {
    calls.push({ cmd, args });
    if (cmd === "replace_face_color") {
      return {
        updatedFaces: [0, 2],
        updatedColors: [
          [40, 167, 69, 255],
          [40, 167, 69, 255],
        ],
        faces: 2,
        regions: 2,
      };
    }
    return {};
  });
  return { mockInvoke, calls };
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: mockInvoke }));

// 3 faces RED (labels 1, 2, 2), 2 faces BLUE (labels 3, 3), 1 face default.
const RED: [number, number, number, number] = [139, 69, 19, 255];
const GREEN: [number, number, number, number] = [40, 167, 69, 255];

const USED = [
  { color: RED, faces: 3, regions: 2, isDefault: false },
  { color: [30, 144, 255, 255] as [number, number, number, number], faces: 2, regions: 1, isDefault: false },
];

function meshFixture(): MeshData {
  const one = (c: [number, number, number, number]) => [c[0], c[1], c[2], c[3]];
  return {
    vertices: [],
    faces: [],
    faceColors: [
      ...one(RED),
      ...one(RED),
      ...one(RED),
      ...one([30, 144, 255, 255]),
      ...one([30, 144, 255, 255]),
      ...one([138, 138, 138, 255]),
    ],
    segmentLabels: [1, 2, 2, 3, 3, 4],
    bbox: { min: [0, 0, 0], max: [1, 1, 1] },
    faceCount: 6,
    segments: [],
  };
}

beforeEach(() => {
  calls.length = 0;
  useAppStore.setState({
    language: "zh",
    meshData: meshFixture(),
    currentColor: [40, 167, 69, 255],
    viewSelectedRegions: [],
    pendingPaintResult: null,
    statusMessage: "",
  });
});

// ── Issue #29: the replace section next to the used-colour swatches ──
describe("ReplaceColourSection", () => {
  it("从 → pick a used swatch → live preview shows the dry-run counts", async () => {
    const user = userEvent.setup();
    render(<ReplaceColourSection used={USED} />);

    // Open the source picker, pick RED.
    await user.click(screen.getByRole("button", { name: /从/ }));
    await user.click(screen.getByTitle(/#8B4513/i));

    // Open the target picker, pick BLUE.
    await user.click(screen.getByRole("button", { name: /到/ }));
    await user.click(screen.getByTitle(/#1E90FF/i));

    const preview = screen.getByText(/将把 3 面 · 2 个区域/);
    expect(preview).toBeTruthy();
  });

  it("from == to → the apply button is disabled", async () => {
    const user = userEvent.setup();
    render(<ReplaceColourSection used={USED} />);

    await user.click(screen.getByRole("button", { name: "从" }));
    await user.click(within(screen.getByText(/选择来源颜色/).parentElement as HTMLElement).getByTitle(/#8B4513/i));
    await user.click(screen.getByRole("button", { name: "到" }));
    await user.click(within(screen.getByText(/选择目标颜色/).parentElement as HTMLElement).getByTitle(/#8B4513/i));

    const apply = screen.getByRole("button", { name: "应用" }) as HTMLButtonElement;
    expect(apply.disabled).toBe(true);
    expect(screen.getByText("来源与目标颜色相同")).toBeTruthy();
    expect(calls.some((c) => c.cmd === "replace_face_color")).toBe(false);
  });

  it("default grey as source → refused with the safety hint", async () => {
    const user = userEvent.setup();
    render(<ReplaceColourSection used={[...USED, { color: [138, 138, 138, 255], faces: 1, regions: 1, isDefault: true }]} />);

    await user.click(screen.getByRole("button", { name: "从" }));
    await user.click(within(screen.getByText(/选择来源颜色/).parentElement as HTMLElement).getByTitle(/#8A8A8A/i));
    await user.click(screen.getByRole("button", { name: "到" }));
    await user.click(within(screen.getByText(/选择目标颜色/).parentElement as HTMLElement).getByTitle(/#1E90FF/i));

    const apply = screen.getByRole("button", { name: "应用" }) as HTMLButtonElement;
    expect(apply.disabled).toBe(true);
    expect(screen.getByText(/不能以未上色的默认灰作为来源/)).toBeTruthy();
  });

  it("scope=selected regions is disabled while nothing is selected in the viewport", () => {
    render(<ReplaceColourSection used={USED} />);
    const radio = screen.getByLabelText("仅选中分区") as HTMLInputElement;
    expect(radio.disabled).toBe(true);
    expect(radio.title).toContain("Ctrl");
  });

  it("应用 → one replace_face_color IPC + the result rides the pending bridge + done status", async () => {
    const user = userEvent.setup();
    render(<ReplaceColourSection used={USED} />);

    await user.click(screen.getByRole("button", { name: /从/ }));
    await user.click(screen.getByTitle(/#8B4513/i));
    await user.click(screen.getByRole("button", { name: /到/ }));
    await user.click(screen.getByTitle(/#1E90FF/i));

    await user.click(screen.getByRole("button", { name: "应用" }));

    await waitFor(() =>
      expect(calls.filter((c) => c.cmd === "replace_face_color")).toHaveLength(1)
    );
    expect(calls[0].args).toMatchObject({ from: RED, to: [30, 144, 255, 255], labels: null });

    // The GPU/mirror bridge receives the incremental payload; the histogram
    // refresh rides the same historyVersion bump the bridge triggers.
    const pending = useAppStore.getState().pendingPaintResult;
    expect(pending?.faces).toEqual([0, 2]);
    expect(useAppStore.getState().statusMessage).toContain("已替换 2 面");
    // The applied status carries the source hex (the dry-run → applied trail).
    expect(useAppStore.getState().statusMessage).toContain("8B4513");
  });
});
