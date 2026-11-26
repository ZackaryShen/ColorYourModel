// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { MeshData } from "../../types/mesh";
import { ExportDialog } from "./ExportDialog";

const { invoke, save, export3mf, exportObj } = vi.hoisted(() => ({
  invoke: vi.fn(),
  save: vi.fn(),
  export3mf: vi.fn(),
  exportObj: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save }));
vi.mock("../../hooks/useTauriCommand", () => ({ useTauriCommand: () => ({ export3mf, exportObj }) }));
vi.mock("../TexturePreview/TexturePreviewCanvas", () => ({
  TexturePreviewCanvas: ({ label, converted }: { label: string; converted: MeshData }) =>
    <div aria-label={label}>{converted.faceColors.join(",")}</div>,
}));

const selection = {
  machineId: "snapmaker_u1",
  nozzleDiameter: "0.4",
  processName: "Standard",
  filamentNames: ["PLA", "PLA", "PLA", "PLA"],
  targetSlicer: "snapmaker_orca",
};

const nominalColors = [null, [0, 0, 255], null, [255, 0, 0]];
const red = [255, 0, 0];
const blue = [0, 0, 255];

describe("texture mappings in ExportDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useAppStore.setState({
      language: "en",
      lastExportSelection: null,
      activeTabId: 3,
      tabEpoch: 1,
      saveBusy: false,
      importBusy: false,
      pendingCloseTabId: null,
      pendingImportPath: null,
      isLoading: false,
      meshData: {
        texturePainting: true,
        vertices: [0, 0, 0, 1, 0, 0, 0, 1, 0, 1, 1, 0],
        faces: [0, 1, 2, 1, 3, 2],
        faceColors: [255, 0, 0, 255, 0, 0, 255, 255],
        segmentLabels: [0, 0],
        bbox: { min: [0, 0, 0], max: [1, 1, 0] },
        faceCount: 2,
        segments: [],
      },
    });
    save.mockResolvedValue("/tmp/mapped.3mf");
    export3mf.mockResolvedValue("Exported to /tmp/mapped.3mf");
    exportObj.mockResolvedValue("Exported to /tmp/legacy.obj");
    invoke.mockImplementation(async (name: string) => {
      if (name === "list_export_presets") return [{
        id: selection.machineId,
        vendor: "Snapmaker",
        name: "U1",
        nozzles: ["0.4"],
        slotsByNozzle: { "0.4": 4 },
        targetSlicers: [selection.targetSlicer],
        processes: [{ name: selection.processName, nozzles: ["0.4"], layerHeight: "0.2" }],
        filaments: [{ name: "PLA", nozzles: ["0.4"], material: "PLA", vendor: null }],
      }];
      if (name === "texture_mapping_preview") return {
        revision: "preview-revision",
        rows: [{ rgb: blue, faceCount: 1 }, { rgb: red, faceCount: 1 }],
        accepted: { selection, nominalColors, assignments: [{ rgb: red, slot: 4 }] },
      };
      if (name === "export_palette_preview") return { flat: ["#0000FF", "#FF0000"], gradientBySlotCount: {} };
      if (name === "suggest_texture_mapping") return {
        slots: nominalColors.map((color, i) => ({
          slot: i + 1, nominalColor: color, displayColor: color ?? [138, 138, 138],
        })),
        assignments: [{ rgb: red, slot: 2 }, { rgb: blue, slot: 2 }],
      };
      throw new Error(`Unexpected command ${name}`);
    });
  });

  afterEach(cleanup);

  it("keeps an accepted assignment and fills only unassigned rows", async () => {
    render(<ExportDialog onClose={() => {}} />);
    const redSlot = await screen.findByRole("combobox", { name: "Slot for #FF0000" }) as HTMLSelectElement;
    const blueSlot = screen.getByRole("combobox", { name: "Slot for #0000FF" }) as HTMLSelectElement;
    expect(redSlot.value).toBe("4");
    expect(blueSlot.value).toBe("");

    const confirm = screen.getByRole("button", { name: "Confirm and export" }) as HTMLButtonElement;
    expect(confirm.disabled).toBe(true);
    const suggest = await screen.findByRole("button", { name: "Fill unassigned colours with suggestions" }) as HTMLButtonElement;
    await waitFor(() => expect(suggest.disabled).toBe(false));
    fireEvent.click(suggest);
    expect(redSlot.value).toBe("4");
    expect(blueSlot.value).toBe("2");
    expect(confirm.disabled).toBe(false);
  });

  it("previews many model colours on one physical spool and sends the exact assignment", async () => {
    save.mockResolvedValue("/tmp/mapped.3mf.3mf");
    render(<ExportDialog onClose={() => {}} />);
    const blueSlot = await screen.findByRole("combobox", { name: "Slot for #0000FF" });
    expect(invoke).toHaveBeenCalledWith("texture_mapping_preview", { tabId: 3 });
    expect(screen.getByText(/exports the active texture plate/)).toBeTruthy();
    fireEvent.change(blueSlot, { target: { value: "4" } });

    await waitFor(() => expect(screen.getByLabelText("Filament assignment preview").textContent)
      .toBe("255,0,0,255,255,0,0,255"));
    const confirm = screen.getByRole("button", { name: "Confirm and export" }) as HTMLButtonElement;
    await waitFor(() => expect(confirm.disabled).toBe(false));
    fireEvent.click(confirm);
    await waitFor(() => expect(export3mf).toHaveBeenCalledWith("/tmp/mapped.3mf.3mf", selection, false, {
      revision: "preview-revision",
      mapping: {
        selection,
        nominalColors,
        assignments: [{ rgb: blue, slot: 4 }, { rgb: red, slot: 4 }],
      },
    }));
  });

  it("drops a mapped export if the source plate generation changes while the picker is open", async () => {
    let settle!: (path: string) => void;
    save.mockReturnValue(new Promise((resolve) => { settle = resolve; }));
    render(<ExportDialog onClose={() => {}} />);
    fireEvent.change(await screen.findByRole("combobox", { name: "Slot for #0000FF" }), { target: { value: "4" } });
    const confirm = screen.getByRole<HTMLButtonElement>("button", { name: "Confirm and export" });
    await waitFor(() => expect(confirm.disabled).toBe(false));
    fireEvent.click(confirm);
    await waitFor(() => expect(save).toHaveBeenCalled());
    act(() => { useAppStore.setState({ tabEpoch: 2 }); });
    await act(async () => settle("/tmp/stale.3mf"));
    expect(export3mf).not.toHaveBeenCalled();
    expect(useAppStore.getState().saveBusy).toBe(false);
  });

  it("exports a normal mesh to OBJ without a texture mapping", async () => {
    const meshData = useAppStore.getState().meshData!;
    useAppStore.setState({ meshData: { ...meshData, texturePainting: false } });
    save.mockResolvedValue("/tmp/legacy.obj.obj");
    render(<ExportDialog onClose={() => {}} />);

    await screen.findByRole("checkbox", { name: "Select slot 2" });
    fireEvent.click(screen.getByRole("button", { name: "OBJ (with colour)" }));
    fireEvent.click(screen.getByRole("button", { name: "Choose location & export" }));
    await waitFor(() => expect(exportObj).toHaveBeenCalledWith("/tmp/legacy.obj.obj"));
    expect(invoke).not.toHaveBeenCalledWith("texture_mapping_preview");
    expect(export3mf).not.toHaveBeenCalled();
  });

  it("renders grouped faces from the export preview's row indexes", async () => {
    const fixture = invoke.getMockImplementation()!;
    invoke.mockImplementation(async (command: string, args: unknown) => {
      const result = await fixture(command, args);
      return command === "texture_mapping_preview"
        ? { ...result, grouping: { sourceColorCount: 27, faceRows: [0, 1] } }
        : result;
    });
    render(<ExportDialog onClose={() => {}} />);
    await screen.findByRole("combobox", { name: "Slot for #FF0000" });
    await waitFor(() => expect(screen.getByLabelText("Filament assignment preview").textContent)
      .toBe("0,0,255,255,255,0,0,255"));
    expect(screen.getByText("Grouped 27 model colours into 2 export colours.")).toBeTruthy();
  });

  it("keeps manual slots and the preview when clicking the texture export theme switch", async () => {
    useAppStore.setState({ theme: "dark" });
    const user = userEvent.setup();
    render(<ExportDialog onClose={() => {}} />);
    const blueSlot = await screen.findByRole<HTMLSelectElement>("combobox", { name: "Slot for #0000FF" });
    await user.selectOptions(blueSlot, "4");
    const preview = screen.getByLabelText("Filament assignment preview");
    await user.click(screen.getByRole("button", { name: "Switch to light theme" }));

    expect(useAppStore.getState().theme).toBe("light");
    expect(screen.getByRole("button", { name: "Switch to dark theme" })).toBeTruthy();
    expect(blueSlot.value).toBe("4");
    expect(screen.getByLabelText("Filament assignment preview")).toBe(preview);
    expect(invoke.mock.calls.filter(([command]) => command === "texture_mapping_preview")).toHaveLength(1);
  });
});
