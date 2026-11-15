import { describe, it, expect } from "vitest";
import { importStatusMessage } from "./useImportStl";
import { translate } from "../i18nDict";

const t = (key: string, ...args: (string | number)[]): string =>
  translate(key, "en", ...args);

describe("importStatusMessage", () => {
  it("surfaces a dropped engineering state over the multi-plate notice", () => {
    expect(
      importStatusMessage(
        { engineeringState: "dropped", plateIndex: 1, plateCount: 3 },
        t
      )
    ).toBe(
      "Engineering data does not match the geometry; imported as a plain 3MF"
    );
  });

  it("surfaces a dropped undo history when the engineering state restored", () => {
    expect(
      importStatusMessage(
        { engineeringState: "restored", historyDropped: true },
        t
      )
    ).toBe("Undo history could not be loaded; starting with an empty history");
  });

  it("keeps the engineering drop notice ahead of the history notice", () => {
    expect(
      importStatusMessage(
        { engineeringState: "dropped", historyDropped: true },
        t
      )
    ).toBe(
      "Engineering data does not match the geometry; imported as a plain 3MF"
    );
  });

  it("keeps the multi-plate message for a normal multi-plate import", () => {
    expect(
      importStatusMessage(
        { engineeringState: "absent", plateIndex: 1, plateCount: 2 },
        t
      )
    ).toBe("Imported plate 1 of 2 (multi-plate paging comes later)");
  });

  it("keeps the plain import message for STL, single-plate and restored files", () => {
    for (const data of [
      {},
      { engineeringState: "absent" as const },
      { engineeringState: "restored" as const, historyDropped: false },
      { plateIndex: 1, plateCount: 1 },
    ]) {
      expect(importStatusMessage(data, t)).toBe("Import complete");
    }
  });
});
