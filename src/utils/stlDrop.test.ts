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
    for (const path of ["C:\\m\\part.obj", "C:\\m\\part.cym", "C:\\m\\README"]) {
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

  it("accepts .3mf next to .stl and rejects .cym", () => {
    expect(planFileDrop(["C:\\m\\a.3mf"], false)).toEqual({ action: "import", path: "C:\\m\\a.3mf" });
    expect(planFileDrop(["C:\\m\\a.stl"], false)).toEqual({ action: "import", path: "C:\\m\\a.stl" });
    expect(planFileDrop(["C:\\m\\a.cym"], false)).toEqual({ action: "reject", reason: "extension" });
  });

  it("treats an empty drop as a multi-file rejection (never imports)", () => {
    expect(planFileDrop([], false)).toEqual({ action: "reject", reason: "multi" });
  });
});
