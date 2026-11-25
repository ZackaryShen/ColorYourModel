import { describe, it, expect } from "vitest";
import { importStatusMessage } from "./useImportStl";
import { translate } from "../i18nDict";

const t = (key: string, ...args: (string | number)[]): string =>
  translate(key, "en", ...args);

describe("importStatusMessage", () => {
  it("keeps the multi-plate limitation visible alongside a dropped engineering state", () => {
    expect(
      importStatusMessage(
        { engineeringState: "dropped", plateIndex: 1, plateCount: 3 },
        t
      )
    ).toBe(
      "Engineering data does not match the geometry; imported as a plain 3MF · Plate 1 of 3; switch plates using the tabs above"
    );
  });

  it("keeps the multi-plate limitation visible alongside a dropped history", () => {
    const message = importStatusMessage(
      { engineeringState: "restored", historyDropped: true, plateIndex: 1, plateCount: 2 },
      t
    );
    expect(message).toContain("undo history was not restored");
    expect(message).toContain("Plate 1 of 2");
  });

  it("surfaces a dropped undo history when the engineering state restored", () => {
    expect(
      importStatusMessage(
        { engineeringState: "restored", historyDropped: true },
        t
      )
    ).toBe("Some or all undo history was not restored; available steps were kept");
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
    ).toBe("Plate 1 of 2; switch plates using the tabs above");
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
