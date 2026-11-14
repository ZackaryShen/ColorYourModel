import { describe, it, expect } from "vitest";
import { displaySegmentName, isDefaultSegmentName } from "./segmentLabel";

/** Stand-in for useT() — returns the key plus args so assertions can see
 *  exactly what was asked for without loading the real dictionary. */
const fakeT = (key: string, ...args: (string | number)[]) => `${key}(${args.join(",")})`;

describe("displaySegmentName", () => {
  it("localises the backend default name", () => {
    expect(displaySegmentName("Region 3", fakeT)).toBe("segments.defaultName(3)");
  });

  it("leaves a user rename untouched", () => {
    expect(displaySegmentName("头发", fakeT)).toBe("头发");
    expect(displaySegmentName("Region hair", fakeT)).toBe("Region hair");
  });

  it("leaves anything that only looks similar untouched", () => {
    // The backend mints exactly "Region <n>"; these must not be rewritten.
    expect(displaySegmentName("Region 3 copy", fakeT)).toBe("Region 3 copy");
    expect(displaySegmentName("Region 3", fakeT)).not.toContain("3 copy");
    expect(displaySegmentName("region 3", fakeT)).toBe("region 3");
    expect(displaySegmentName("", fakeT)).toBe("");
  });
});

describe("isDefaultSegmentName", () => {
  it("recognises the backend pattern only", () => {
    expect(isDefaultSegmentName("Region 1")).toBe(true);
    expect(isDefaultSegmentName("Region 120")).toBe(true);
    expect(isDefaultSegmentName("Region")).toBe(false);
    expect(isDefaultSegmentName("Region one")).toBe(false);
    expect(isDefaultSegmentName("我的分区")).toBe(false);
  });
});
