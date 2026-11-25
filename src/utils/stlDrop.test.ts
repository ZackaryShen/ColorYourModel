import { describe, it, expect } from "vitest";
import { normalizeDroppedPath, planFileDrop } from "./stlDrop";

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

describe("normalizeDroppedPath", () => {
  it("leaves a plain path alone", () => {
    expect(normalizeDroppedPath("/Users/zackary/Downloads/梁圣.3mf")).toBe(
      "/Users/zackary/Downloads/梁圣.3mf",
    );
  });

  it("decodes a percent-encoded file URL so a Chinese name keeps its extension", () => {
    // The macOS / Linux drag-drop bridge can hand over a file:// URL with the
    // path percent-encoded; before this, the extension test rejected it.
    const raw = "file:///Users/zackary/Downloads/%E6%A2%81%E5%9C%A3.3mf";
    expect(planFileDrop([raw], false)).toEqual({
      action: "import",
      path: "/Users/zackary/Downloads/梁圣.3mf",
    });
  });

  it("strips the leading slash of a Windows file URL", () => {
    expect(normalizeDroppedPath("file:///C:/models/part.3mf")).toBe("C:/models/part.3mf");
  });

  it("keeps the raw path when the escape sequence is malformed", () => {
    expect(normalizeDroppedPath("/tmp/100%broken.3mf")).toBe("/tmp/100%broken.3mf");
  });
});

it("preserves percent escapes and leading spaces in native filenames", () => {
  expect(normalizeDroppedPath("/tmp/part%20literal.3mf")).toBe("/tmp/part%20literal.3mf");
  expect(normalizeDroppedPath(" /tmp/name.3mf")).toBe(" /tmp/name.3mf");
});
it("accepts localhost file URLs as absolute local paths", () => {
  expect(normalizeDroppedPath("file://localhost/tmp/a%20b.3mf")).toBe("/tmp/a b.3mf");
});
