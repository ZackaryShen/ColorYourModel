import { describe, it, expect, vi, beforeEach } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { Toolbar } from "./Toolbar";
import { useAppStore } from "../../store/appStore";

// The toolbar mounts Tauri progress listeners at render time; outside the
// WebView those modules must be stubbed or the effect throws.
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => ({})) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));

/** Title attribute is the button's accessible identity here (icon-only bar). */
const titles = () =>
  Array.from(document.querySelectorAll("button[title]")).map(
    (b) => b.getAttribute("title") ?? ""
  );

beforeEach(() => {
  // Chinese strings keep the assertions readable; the labels under test are
  // the tool tooltips, not the UI language default.
  useAppStore.setState({ language: "zh", activeTool: "view" });
});

describe("Toolbar tool groups (0.2.0-P1 req #6)", () => {
  it("renders every tool exactly once — Seed included", () => {
    render(<Toolbar />);
    // Regression: the groups were built with TOOL_KEYS.slice() indices. Adding
    // the gradient tool shifted every later index, so the segment group became
    // [Eraser, Segment, Lasso] — Eraser twice and Seed missing, i.e. the seed
    // panel became unreachable from the bar.
    expect(screen.getByTitle(/种子分区/)).toBeTruthy();
    expect(screen.getByTitle(/橡皮/)).toBeTruthy();
    expect(screen.getAllByTitle(/橡皮/)).toHaveLength(1);
    expect(screen.getByTitle(/渐变画笔/)).toBeTruthy();
    expect(screen.getByTitle(/选点套索/)).toBeTruthy();
    // View is always visible outside the groups.
    expect(screen.getByTitle(/查看\/导航/)).toBeTruthy();
    expect(titles().filter((t) => t.includes("橡皮"))).toHaveLength(1);
  });

  it("chevron collapses a group that does not own the active tool", () => {
    useAppStore.setState({ activeTool: "view" });
    render(<Toolbar />);
    const chevrons = screen.getAllByRole("button", { name: /涂色工具|分区工具/ });
    expect(chevrons.length).toBe(2);
    // Collapse the paint group while View is active: its tools disappear.
    fireEvent.click(screen.getByRole("button", { name: /涂色工具/ }));
    expect(screen.queryByTitle(/渐变画笔/)).toBeNull();
    // The segment group is untouched, so Seed stays reachable.
    expect(screen.getByTitle(/种子分区/)).toBeTruthy();
  });
});
