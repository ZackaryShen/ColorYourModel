import { describe, it, expect } from "vitest";
import {
  ALGORITHM_KINDS,
  EXPERIMENTAL_ALGORITHM_KINDS,
  isAlgorithmKind,
  isExperimentalKind,
  visibleAlgorithmKinds,
} from "./segment";

describe("experimental algorithm kinds — 0.2.0 P0 feature flag", () => {
  it("the experimental list is a non-empty subset of ALGORITHM_KINDS", () => {
    expect(EXPERIMENTAL_ALGORITHM_KINDS.length).toBeGreaterThan(0);
    for (const k of EXPERIMENTAL_ALGORITHM_KINDS) {
      expect(isAlgorithmKind(k)).toBe(true);
      expect(isExperimentalKind(k)).toBe(true);
    }
  });

  it("non-experimental kinds are not flagged", () => {
    for (const k of ALGORITHM_KINDS) {
      if (!(EXPERIMENTAL_ALGORITHM_KINDS as string[]).includes(k)) {
        expect(isExperimentalKind(k)).toBe(false);
      }
    }
    expect(isExperimentalKind("curvatureKMeans")).toBe(false);
  });

  it("flag off hides exactly the experimental kinds; flag on shows everything", () => {
    const hidden = visibleAlgorithmKinds(false);
    const shown = visibleAlgorithmKinds(true);

    expect(shown).toEqual(ALGORITHM_KINDS);
    expect(hidden.length).toBe(ALGORITHM_KINDS.length - EXPERIMENTAL_ALGORITHM_KINDS.length);
    for (const k of EXPERIMENTAL_ALGORITHM_KINDS) {
      expect(hidden).not.toContain(k);
    }
    // The documented iter-45 default must stay reachable with the flag off.
    expect(hidden).toContain("curvatureKMeans");
  });
});
