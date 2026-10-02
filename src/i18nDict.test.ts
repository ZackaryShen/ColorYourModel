import { describe, it, expect } from "vitest";
import { I18N_DICT, translate, translateError } from "./i18nDict";

// Permanent regression guards for the bilingual dictionary: a key missing
// either language, or a {0}/{1} placeholder present in one language but not
// the other, renders literally ("{0}") or leaks the wrong language into the
// UI — both shipped before (the 2026-10-01 full i18n audit found several).
const PLACEHOLDER = /\{(\d+)\}/g;

const placeholders = (s: string): string =>
  Array.from(s.matchAll(PLACEHOLDER))
    .map((m) => m[1])
    .sort()
    .join(",");

describe("i18nDict", () => {
  const entries = Object.entries(I18N_DICT);

  it("is non-empty", () => {
    expect(entries.length).toBeGreaterThan(200);
  });

  it("ships a non-empty zh and en text for every key", () => {
    expect(entries.length).toBeGreaterThan(0);
    for (const [key, entry] of entries) {
      expect(typeof entry.zh, `zh type of "${key}"`).toBe("string");
      expect(entry.zh.trim().length, `zh text of "${key}"`).toBeGreaterThan(0);
      expect(typeof entry.en, `en type of "${key}"`).toBe("string");
      expect(entry.en.trim().length, `en text of "${key}"`).toBeGreaterThan(0);
    }
  });

  it("keeps {0}/{1} placeholders aligned between zh and en", () => {
    for (const [key, entry] of entries) {
      expect(placeholders(entry.en), `placeholders of "${key}"`).toBe(
        placeholders(entry.zh)
      );
    }
  });

  it("substitutes positional args into both languages", () => {
    expect(translate("export.success", "zh", "a.3mf")).toBe("导出成功：a.3mf");
    expect(translate("export.success", "en", "a.3mf")).toBe("Exported: a.3mf");
  });

  it("returns unknown keys and unknown backend errors verbatim", () => {
    expect(translate("nope.missing.key", "en")).toBe("nope.missing.key");
    expect(translateError("something exploded in a novel way", "zh")).toBe(
      "something exploded in a novel way"
    );
  });

  it("maps known backend error needles to dict keys", () => {
    // The needle substring must localise via translateError, both languages.
    expect(translateError("backend: mesh has no faces", "en")).toBe(
      translate("err.meshNoFaces", "en")
    );
    expect(translateError("backend: mesh has no faces", "zh")).toBe(
      translate("err.meshNoFaces", "zh")
    );
  });
});
