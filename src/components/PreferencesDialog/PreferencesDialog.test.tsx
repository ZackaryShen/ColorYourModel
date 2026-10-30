import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PreferencesDialog } from "./PreferencesDialog";
import { useAppStore } from "../../store/appStore";
import { DEFAULT_EXPERIMENTAL_FEATURES } from "../../experimentalFeatures";

describe("PreferencesDialog — experimental-feature toggles", () => {
  beforeEach(() => {
    useAppStore.setState({
      language: "zh",
      experimentalFeatures: { ...DEFAULT_EXPERIMENTAL_FEATURES },
      planarRegions: [],
      multiviewRegions: [],
      crossSectionRegions: [],
    } as Partial<ReturnType<typeof useAppStore.getState>>);
  });

  const checkboxes = () => screen.queryAllByRole("checkbox");

  it("renders one unchecked checkbox per registry entry (default all hidden)", () => {
    render(<PreferencesDialog onClose={() => {}} />);
    const boxes = checkboxes();
    expect(boxes).toHaveLength(4);
    for (const box of boxes) {
      expect((box as HTMLInputElement).checked).toBe(false);
    }
  });

  it("toggling a checkbox writes straight through to the store", async () => {
    const user = userEvent.setup();
    render(<PreferencesDialog onClose={() => {}} />);
    await user.click(checkboxes()[0]!);
    expect(useAppStore.getState().experimentalFeatures.seedDetectPlanar).toBe(
      true,
    );
    expect(
      useAppStore.getState().experimentalFeatures.seedWeightSliders,
    ).toBe(false);
  });

  it("unchecking a detector clears its region pool (no dangling overlays)", async () => {
    // Simulate: the user detected planar regions while the feature was on,
    // then turns the feature off in the dialog.
    useAppStore.setState({
      experimentalFeatures: {
        ...DEFAULT_EXPERIMENTAL_FEATURES,
        seedDetectPlanar: true,
      },
      planarRegions: [
        {
          seed: { point: [0, 0, 0], faceIndex: 0 },
          faceIndices: [0, 1, 2],
        },
      ] as never,
    });
    const user = userEvent.setup();
    render(<PreferencesDialog onClose={() => {}} />);
    await user.click(checkboxes()[0]!);
    expect(useAppStore.getState().planarRegions).toHaveLength(0);
    expect(
      useAppStore.getState().experimentalFeatures.seedDetectPlanar,
    ).toBe(false);
  });

  it("backdrop click and the × button both close via onClose", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    const { container } = render(<PreferencesDialog onClose={onClose} />);
    // Backdrop = the overlay div itself (first child of the container).
    await user.click(container.firstChild as Element);
    expect(onClose).toHaveBeenCalledTimes(1);
    await user.click(screen.getByRole("button", { name: "关闭偏好设置" }));
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});
