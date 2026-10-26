import { describe, it, expect } from "vitest";
import { planFileDrop } from "./stlDrop";

// Issue #21: dropping files onto the window routes through the SAME import
// pipeline as the 📂 button. `planFileDrop` is the routing decision those
// events funnel through: one file with a supported extension imports (with a
// replace confirmation when a model is already loaded), everything else is
// rejected with a status-bar message instead of being silently swallowed.

describe("planFileDrop", () => {
  it("imports a single .stl into an empty scene, passing the path through verbatim", () => {
    const path = "C:\\Users\\me\\3D Models\\part v2.stl";
    expect(planFileDrop([path], false)).toEqual({ action: "import", path });
  });

  it("asks for confirmation when a model is already loaded", () => {
    const path = "C:\\models\\part.stl";
    expect(planFileDrop([path], true)).toEqual({
      action: "confirm-replace",
      path,
    });
  });

  it("rejects a multi-file drop regardless of extensions", () => {
    expect(planFileDrop(["a.stl", "b.stl"], false)).toEqual({
      action: "reject",
      reason: "multi",
    });
  });

  it("rejects files whose extension is not part of the import pipeline", () => {
    for (const path of ["C:\\m\\part.obj", "C:\\m\\part.3mf", "C:\\m\\README"]) {
      expect(planFileDrop([path], false)).toEqual({
        action: "reject",
        reason: "extension",
      });
    }
  });

  it("matches extensions case-insensitively (Windows filenames)", () => {
    expect(planFileDrop(["C:\\m\\PART.STL"], false)).toEqual({
      action: "import",
      path: "C:\\m\\PART.STL",
    });
    expect(planFileDrop(["C:\\m\\part.Stl"], true)).toEqual({
      action: "confirm-replace",
      path: "C:\\m\\part.Stl",
    });
  });

  it("routes a .cym project through the same format-aware pipeline", () => {
    expect(planFileDrop(["C:\\p\\project.cym"], false)).toEqual({
      action: "import",
      path: "C:\\p\\project.cym",
    });
    expect(planFileDrop(["C:\\p\\project.cym"], true)).toEqual({
      action: "confirm-replace",
      path: "C:\\p\\project.cym",
    });
  });

  it("treats an empty drop as a multi-file rejection (never imports)", () => {
    expect(planFileDrop([], false)).toEqual({ action: "reject", reason: "multi" });
  });
});
