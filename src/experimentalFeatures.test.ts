import { describe, it, expect } from "vitest";
import {
  DEFAULT_EXPERIMENTAL_FEATURES,
  EXPERIMENTAL_FEATURES,
  isExperimentalFeatureId,
  sanitizeExperimentalFeatures,
} from "./experimentalFeatures";

describe("experimentalFeatures registry", () => {
  it("every feature defaults to false (hidden until opted in)", () => {
    for (const f of EXPERIMENTAL_FEATURES) {
      expect(DEFAULT_EXPERIMENTAL_FEATURES[f.id], f.id).toBe(false);
    }
  });

  it("registry ids are unique and match the id union", () => {
    const ids = EXPERIMENTAL_FEATURES.map((f) => f.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const id of ids) expect(isExperimentalFeatureId(id)).toBe(true);
  });

  it("isExperimentalFeatureId rejects unknown and non-string values", () => {
    expect(isExperimentalFeatureId("seedDetectEye")).toBe(false);
    expect(isExperimentalFeatureId(undefined)).toBe(false);
    expect(isExperimentalFeatureId(42)).toBe(false);
  });
});

describe("sanitizeExperimentalFeatures (rehydrate whitelist)", () => {
  it("non-object payloads degrade to the in-code defaults", () => {
    expect(sanitizeExperimentalFeatures(null)).toEqual(
      DEFAULT_EXPERIMENTAL_FEATURES,
    );
    expect(sanitizeExperimentalFeatures(undefined)).toEqual(
      DEFAULT_EXPERIMENTAL_FEATURES,
    );
    expect(sanitizeExperimentalFeatures("garbage")).toEqual(
      DEFAULT_EXPERIMENTAL_FEATURES,
    );
    expect(sanitizeExperimentalFeatures(123)).toEqual(
      DEFAULT_EXPERIMENTAL_FEATURES,
    );
  });

  it("claims known ids with boolean values, drops everything else", () => {
    const result = sanitizeExperimentalFeatures({
      seedDetectPlanar: true,
      seedWeightSliders: false,
      seedEyeDetect: true,
      seedDetectEye: true, // unknown id from a newer build → dropped
      seedDetectMultiview: "yes", // wrong type → dropped
      seedDetectCrossSection: 1, // wrong type → dropped
    });
    expect(result).toEqual({
      seedDetectPlanar: true,
      seedDetectMultiview: false,
      seedDetectCrossSection: false,
      seedWeightSliders: false,
      seedEyeDetect: true,
    });
  });

  it("never lets a partial payload disable the default-hidden contract", () => {
    // An empty object (e.g. a future build removed the field) must not
    // suddenly enable features: everything stays false.
    expect(sanitizeExperimentalFeatures({})).toEqual({
      seedDetectPlanar: false,
      seedDetectMultiview: false,
      seedDetectCrossSection: false,
      seedWeightSliders: false,
      seedEyeDetect: false,
    });
  });
});
