import { beforeEach, describe, expect, it } from "vitest";
import {
  isVersionSkipped,
  isValidSkippedVersion,
  shouldAutoCheckNow,
  useUpdateStore,
  type UpdateInfo,
} from "./updateStore";

/**
 * Pure-policy and state-machine tests for the update module. No Tauri:
 * updateStore performs no Tauri calls at import time, and the actions under
 * test here either don't reach the network (guards, close/skip actions) or
 * are deliberately NOT driven into invoke-taking branches.
 */

const info: UpdateInfo = {
  current: "0.1.1",
  latest: "0.2.0",
  notes: null,
  releaseUrl: "https://github.com/ZackaryShen/ColorYourModel/releases/tag/v0.2.0",
  canAutoInstall: true,
  downloadUrl: "https://github.com/.../ColorYourModel_0.2.0_x64-setup.exe",
  assetSize: 3_590_475,
};

const reset = () =>
  useUpdateStore.setState({
    autoCheckEnabled: true,
    skippedVersion: null,
    phase: "idle",
    info: null,
    errorReason: null,
    errorFrom: null,
    progress: { received: 0, total: 0 },
  });

beforeEach(reset);

describe("shouldAutoCheckNow — startup auto-check gate", () => {
  it("never auto-checks in dev builds (dev version line ≠ release line)", () => {
    expect(shouldAutoCheckNow({ isDev: true, autoCheckEnabled: true })).toBe(false);
  });

  it("respects the persisted opt-out (不再提醒)", () => {
    expect(shouldAutoCheckNow({ isDev: false, autoCheckEnabled: false })).toBe(false);
  });

  it("checks on a normal production launch", () => {
    expect(shouldAutoCheckNow({ isDev: false, autoCheckEnabled: true })).toBe(true);
  });
});

describe("isVersionSkipped — 跳过此版本 semantics", () => {
  it("matches the exact normalized version", () => {
    expect(isVersionSkipped("0.2.0", "0.2.0")).toBe(true);
  });

  it("a newer release prompts again after a skip", () => {
    expect(isVersionSkipped("0.2.1", "0.2.0")).toBe(false);
  });

  it("no skip recorded → never skipped", () => {
    expect(isVersionSkipped("0.2.0", null)).toBe(false);
  });
});

describe("isValidSkippedVersion — persisted-form validation", () => {
  it("accepts the normalized (no-v) form only", () => {
    expect(isValidSkippedVersion("0.2.0")).toBe(true);
    // The store compares with ===; a v-prefixed stray would make the skip
    // comparison permanently false (REFUTE M6) — rejected here instead.
    expect(isValidSkippedVersion("v0.2.0")).toBe(false);
    expect(isValidSkippedVersion("garbage")).toBe(false);
    expect(isValidSkippedVersion(42)).toBe(false);
    expect(isValidSkippedVersion(null)).toBe(false);
  });
});

describe("updateStore state machine guards", () => {
  it("manual check is ignored while a download owns the flow (REFUTE M4)", () => {
    useUpdateStore.setState({ phase: "downloading" });
    useUpdateStore.getState().startManualCheck();
    expect(useUpdateStore.getState().phase).toBe("downloading");
  });

  it("manual check is ignored while installing", () => {
    useUpdateStore.setState({ phase: "installing" });
    useUpdateStore.getState().startManualCheck();
    expect(useUpdateStore.getState().phase).toBe("installing");
  });

  it("auto check is ignored unless idle", () => {
    useUpdateStore.setState({ phase: "available" });
    useUpdateStore.getState().startAutoCheck();
    expect(useUpdateStore.getState().phase).toBe("available");
  });

  it("later() closes the available dialog without touching prefs", () => {
    useUpdateStore.setState({ phase: "available", info });
    useUpdateStore.getState().later();
    const s = useUpdateStore.getState();
    expect(s.phase).toBe("idle");
    expect(s.skippedVersion).toBeNull();
    expect(s.autoCheckEnabled).toBe(true);
  });

  it("skipVersion() persists the NORMALIZED version and closes", () => {
    useUpdateStore.setState({ phase: "available", info });
    useUpdateStore.getState().skipVersion();
    const s = useUpdateStore.getState();
    expect(s.skippedVersion).toBe("0.2.0");
    expect(s.phase).toBe("idle");
  });

  it("neverRemind() disables the startup check and closes", () => {
    useUpdateStore.setState({ phase: "available", info });
    useUpdateStore.getState().neverRemind();
    const s = useUpdateStore.getState();
    expect(s.autoCheckEnabled).toBe(false);
    expect(s.phase).toBe("idle");
  });

  it("updateNow() refuses to fire outside available/download-error phases", () => {
    useUpdateStore.setState({ phase: "idle", info });
    useUpdateStore.getState().updateNow();
    // Must not have entered downloading (no invoke in jsdom — phase proves it).
    expect(useUpdateStore.getState().phase).toBe("idle");
  });

  it("updateNow() refuses without a downloadable asset (page-fallback case)", () => {
    useUpdateStore.setState({ phase: "available", info: { ...info, downloadUrl: null } });
    useUpdateStore.getState().updateNow();
    expect(useUpdateStore.getState().phase).toBe("available");
  });

  it("installNow() refuses unless ready", () => {
    useUpdateStore.setState({ phase: "downloading" });
    useUpdateStore.getState().installNow();
    expect(useUpdateStore.getState().phase).toBe("downloading");
  });

  it("discardAndClose() only acts on ready", () => {
    useUpdateStore.setState({ phase: "available" });
    useUpdateStore.getState().discardAndClose();
    expect(useUpdateStore.getState().phase).toBe("available");
  });
});
