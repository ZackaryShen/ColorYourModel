import { useEffect } from "react";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { TextureImportSession } from "../../hooks/useTextureImport";
import { useAppStore } from "../../store/appStore";
import { TexturePreviewDialog } from "./TexturePreviewDialog";

vi.mock("./TexturePreviewCanvas", () => ({
  TexturePreviewCanvas: ({ original, onReady }: {
    original: unknown;
    onReady: () => void;
  }) => {
    useEffect(onReady, [original]);
    return null;
  },
}));

const session: TextureImportSession = {
  jobId: 1, path: "/models/Tide Vase.glb", colorLimit: 8, phase: "ready", error: null,
  preview: {
    original: {
      vertices: [0, 0, 0, 1, 0, 0, 0, 1, 0],
      faces: [0, 1, 2], cornerUvs: [0, 0, 1, 0, 0, 1],
      imageUrl: "data:image/png;base64,AA==", wrapS: "repeat", wrapT: "repeat",
      baseColorFactor: [1, 1, 1, 1],
    },
    converted: {
      vertices: [0, 0, 0, 1, 0, 0, 0, 1, 0, 0.5, 0.5, 0],
      faces: [0, 1, 3, 0, 3, 2],
      bbox: { min: [0, 0, 0], max: [1, 1, 0] },
      faceCount: 2, faceColors: [255, 0, 0, 255, 0, 0, 255, 255],
      segmentLabels: [0, 0], segments: [],
    },
    palette: [[255, 0, 0], [0, 0, 255]],
  },
};

function renderPreview(active: TextureImportSession = session) {
  const onApply = vi.fn();
  const onCancel = vi.fn();
  render(
    <TexturePreviewDialog session={active} hasModel onApply={onApply} onCancel={onCancel}
      onColorLimit={vi.fn()} onRetry={vi.fn()} />,
  );
  return { onApply, onCancel };
}

beforeEach(() => {
  Object.defineProperties(HTMLDialogElement.prototype, {
    showModal: { configurable: true, value(this: HTMLDialogElement) { this.open = true; } },
    close: { configurable: true, value(this: HTMLDialogElement) { this.open = false; } },
  });
  useAppStore.setState({ language: "en", theme: "dark", isLoading: false, paintDirty: true });
});

describe("texture import preview", () => {
  it("explains a reduced preview and keeps the apply decision available", () => {
    renderPreview({ ...session, preview: { ...session.preview!, warning: "budget" } });
    expect(screen.getByText(/subdivision face budget was reached/i)).toBeTruthy();
    expect((screen.getByRole("button", { name: "Apply and replace model" }) as HTMLButtonElement).disabled).toBe(false);
  });
  it("preserves the selected comparison mode and divider when switching theme", () => {
    renderPreview();
    fireEvent.click(screen.getByRole("button", { name: "Slider" }));
    fireEvent.change(screen.getByRole("slider", { name: "Comparison divider" }), { target: { value: "26" } });
    fireEvent.click(screen.getByRole("button", { name: "Switch to light theme" }));
    expect(useAppStore.getState().theme).toBe("light");
    expect(screen.getByRole("button", { name: "Slider" }).getAttribute("aria-pressed")).toBe("true");
    expect((screen.getByRole("slider", { name: "Comparison divider" }) as HTMLInputElement).value).toBe("26");
    expect(screen.getByText("Tide Vase.glb")).toBeTruthy();
  });

  it("requires an explicit replace action and displays returned mesh counts", () => {
    const { onApply } = renderPreview();
    expect(screen.getByText("1 face")).toBeTruthy();
    expect(screen.getByText("2 faces")).toBeTruthy();
    expect(onApply).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Apply and replace model" }));
    expect(onApply).toHaveBeenCalledOnce();
  });

  it("blocks cancel and apply while the model commit is in flight", () => {
    const { onApply, onCancel } = renderPreview({ ...session, phase: "applying" });
    const dialog = screen.getByRole("dialog");
    act(() => { dialog.dispatchEvent(new Event("cancel", { cancelable: true })); });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    fireEvent.click(screen.getByRole("button", { name: "Apply and replace model" }));
    expect(onCancel).not.toHaveBeenCalled();
    expect(onApply).not.toHaveBeenCalled();
  });
});
