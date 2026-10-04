import { create, type StateCreator } from "zustand";
import { persist, createJSONStorage } from "zustand/middleware";
import { invoke } from "@tauri-apps/api/core";
import { log } from "../utils/logger";

/**
 * Update-module session state + persisted prefs (adversarial plan
 * `.zcode/update-module-plan.md`, REFUTE M2–M6).
 *
 * - This module performs NO Tauri calls at import time (REFUTE M9): `invoke`
 *   only fires from actions, and the download-progress `listen()` lives in
 *   UpdateDialog's useEffect — so importing this file is safe in vitest/jsdom.
 * - The Rust `check_update` never hard-fails; every outcome arrives as a
 *   structured `UpdateCheckResult` (see commands/update.rs for the contract).
 * - Phase transitions: manual checks are refused while a download owns the
 *   flow (downloading/ready/installing) — that mutex is what keeps one
 *   `%TEMP%/cym-update` writer (REFUTE M2/M4).
 * - Version strings cross this boundary in NORMALIZED semver form (no `v`
 *   prefix); `skippedVersion` persists the same form so the skip comparison
 *   is a plain string equality (REFUTE M6).
 */

export type UpdatePhase =
  | "idle" // no dialog
  | "checking" // network check in flight
  | "available" // newer release found, awaiting user decision
  | "downloading" // installer streaming, progress events arriving
  | "ready" // installer on disk, awaiting install confirmation
  | "installing" // installer launched / launching — terminal, dialog locked
  | "error" // last operation failed (manual flows only)
  | "upToDate"; // manual check found nothing newer

export interface UpdateInfo {
  current: string;
  latest: string;
  notes: string | null;
  releaseUrl: string;
  canAutoInstall: boolean;
  downloadUrl: string | null;
  assetSize: number;
}

export type UpdateCheckResult =
  | { status: "up_to_date"; current: string }
  | ({
      status: "available";
    } & UpdateInfo)
  | { status: "check_failed"; reason: string };

/** Phases from which a user-facing check may (re)start. Download-owned
 *  phases are excluded — that's the frontend half of the single-writer rule. */
const CHECKABLE_PHASES: ReadonlySet<UpdatePhase> = new Set([
  "idle",
  "available",
  "upToDate",
  "error",
]);

// ── Pure policy helpers (unit-tested) ───────────────────────────────────────

/** Auto-check gate. Dev builds never check (their version line is unrelated
 *  to published releases); otherwise it's just the persisted preference —
 *  every launch gets ONE silent check (≪ GitHub's 60 req/h limit), so there
 *  is deliberately NO time throttle: 「稍后」 then means "ask again next
 *  launch", which is what users expect (REFUTE M5). */
export function shouldAutoCheckNow(opts: { isDev: boolean; autoCheckEnabled: boolean }): boolean {
  return !opts.isDev && opts.autoCheckEnabled;
}

/** The auto-prompt stays silent for a version the user explicitly skipped.
 *  Manual checks ignore this — asking is consent to see the answer. */
export function isVersionSkipped(latest: string, skippedVersion: string | null): boolean {
  return skippedVersion !== null && latest === skippedVersion;
}

/** Validation for the persisted `skippedVersion`: normalized semver-ish form
 *  only. Anything else degrades to null (no skip) on rehydrate. */
export function isValidSkippedVersion(v: unknown): v is string {
  return typeof v === "string" && /^\d+\.\d+\.\d+/.test(v);
}

// ── Store ───────────────────────────────────────────────────────────────────

interface UpdateStore {
  // Persisted prefs
  /** Master switch for the STARTUP auto-check only; the Help-menu manual
   *  check always works. "不再提醒" writes false; the About box exposes the
   *  toggle so the choice is reversible. */
  autoCheckEnabled: boolean;
  /** NORMALIZED version the user chose to skip (auto prompts only). */
  skippedVersion: string | null;

  // Session state
  phase: UpdatePhase;
  info: UpdateInfo | null;
  errorReason: string | null;
  /** Which step failed — decides what the error dialog's retry button does
   *  (re-check vs re-download). Install failures offer no retry. */
  errorFrom: "check" | "download" | "install" | null;
  progress: { received: number; total: number };

  // Actions
  setAutoCheckEnabled: (v: boolean) => void;
  startAutoCheck: () => void;
  startManualCheck: () => void;
  updateNow: () => void;
  cancelDownload: () => void;
  discardAndClose: () => void;
  installNow: () => void;
  skipVersion: () => void;
  neverRemind: () => void;
  later: () => void;
  dismiss: () => void;
  setProgress: (received: number, total: number) => void;
}

type PersistedUpdatePrefs = Pick<UpdateStore, "autoCheckEnabled" | "skippedVersion">;

const UPDATE_PREFS_KEY = "cym-update-prefs";

// Same probe + in-memory fallback as appStore's pickStorage: the vitest/jsdom
// env (and any storage-less context) must degrade to session-only prefs, never
// crash the store at import time. (Not extracted into a shared module yet —
// noted as backlog in the update-module plan, REFUTE minor-9.)
const memoryBacking = new Map<string, string>();
const memoryStorage = {
  getItem: (name: string) => memoryBacking.get(name) ?? null,
  setItem: (name: string, value: string) => {
    memoryBacking.set(name, value);
  },
  removeItem: (name: string) => {
    memoryBacking.delete(name);
  },
};

function pickStorage(): Storage {
  try {
    const probe = "__cym_update_probe__";
    localStorage.setItem(probe, "1");
    localStorage.removeItem(probe);
    return localStorage;
  } catch {
    return memoryStorage as Storage;
  }
}

function mergeUpdatePrefs(persisted: unknown, current: UpdateStore): UpdateStore {
  const p = (persisted ?? {}) as Partial<PersistedUpdatePrefs>;
  const next: UpdateStore = { ...current };
  if (typeof p.autoCheckEnabled === "boolean") next.autoCheckEnabled = p.autoCheckEnabled;
  if (isValidSkippedVersion(p.skippedVersion)) next.skippedVersion = p.skippedVersion;
  return next;
}

const createUpdateStore: StateCreator<UpdateStore, [], []> = (set, get) => {
  /** Shared body of auto + manual checks. `announce` decides whether
   *  "nothing newer" / "check failed" become visible phases or fall back to
   *  idle silently (auto checks must never nag about network trouble). */
  const runCheck = (announce: boolean) => {
    set({ phase: "checking", errorReason: null, errorFrom: null });
    invoke<UpdateCheckResult>("check_update")
      .then((result) => {
        if (result.status === "available") {
          set({
            info: {
              current: result.current,
              latest: result.latest,
              notes: result.notes,
              releaseUrl: result.releaseUrl,
              canAutoInstall: result.canAutoInstall,
              downloadUrl: result.downloadUrl,
              assetSize: result.assetSize,
            },
            phase: "available",
          });
          return;
        }
        if (result.status === "up_to_date") {
          set(announce ? { phase: "upToDate" } : { phase: "idle" });
          return;
        }
        // check_failed
        if (announce) {
          set({ phase: "error", errorFrom: "check", errorReason: result.reason });
        } else {
          log.warn("update", "auto check failed (silent)", { reason: result.reason });
          set({ phase: "idle" });
        }
      })
      .catch((e) => {
        const reason = `IPC failure: ${String(e)}`;
        if (announce) set({ phase: "error", errorFrom: "check", errorReason: reason });
        else {
          log.warn("update", "auto check failed (silent)", { reason });
          set({ phase: "idle" });
        }
      });
  };

  return {
    autoCheckEnabled: true,
    skippedVersion: null,

    phase: "idle",
    info: null,
    errorReason: null,
    errorFrom: null,
    progress: { received: 0, total: 0 },

    setAutoCheckEnabled: (v) => set({ autoCheckEnabled: v }),

    startAutoCheck: () => {
      if (get().phase !== "idle") return;
      runCheck(false);
    },

    startManualCheck: () => {
      if (!CHECKABLE_PHASES.has(get().phase)) {
        log.info("update", "manual check ignored — updater busy", { phase: get().phase });
        return;
      }
      runCheck(true);
    },

    updateNow: () => {
      const { info, phase, errorFrom } = get();
      const resumable = phase === "available" || (phase === "error" && errorFrom === "download");
      if (!info?.downloadUrl || !resumable) return;
      set({ phase: "downloading", progress: { received: 0, total: info.assetSize } });
      const filename = info.downloadUrl.split("/").pop() ?? "installer.exe";
      invoke<{ outcome: string }>("download_update", {
        url: info.downloadUrl,
        filename,
        expectedSize: info.assetSize,
      })
        .then((result) => {
          if (result.outcome === "done") set({ phase: "ready" });
          else set({ phase: "available" }); // user cancelled — back to the decision point
        })
        .catch((e) => {
          set({ phase: "error", errorFrom: "download", errorReason: String(e) });
        });
    },

    cancelDownload: () => {
      if (get().phase !== "downloading") return;
      invoke("cancel_update_download").catch((e) =>
        log.warn("update", "cancel_update_download failed", { error: String(e) }),
      );
      // The download promise resolves with `cancelled` and updateNow flips the
      // phase back to available — no optimistic set here, so a stuck cancel
      // can't fake state the backend disagrees with.
    },

    discardAndClose: () => {
      if (get().phase !== "ready") return;
      set({ phase: "idle" });
      invoke("discard_update").catch((e) =>
        log.warn("update", "discard_update failed", { error: String(e) }),
      );
    },

    installNow: () => {
      if (get().phase !== "ready") return;
      set({ phase: "installing" });
      invoke("install_update")
        .then(() => import("@tauri-apps/api/window"))
        .then(({ getCurrentWindow }) => getCurrentWindow().destroy())
        .catch((e) => {
          // Destroy already ran if install succeeded but the dynamic import
          // failed (impossible in practice); a launcher failure leaves the
          // app alive with an explanatory error instead of a dead window.
          log.error("update", "install_update failed", { error: String(e) });
          set({ phase: "error", errorFrom: "install", errorReason: String(e) });
        });
    },

    skipVersion: () => {
      const { info } = get();
      if (info && isValidSkippedVersion(info.latest)) set({ skippedVersion: info.latest });
      set({ phase: "idle" });
    },

    neverRemind: () => {
      set({ autoCheckEnabled: false, phase: "idle" });
    },

    later: () => set({ phase: "idle" }),
    dismiss: () => set({ phase: "idle" }),
    setProgress: (received, total) => set({ progress: { received, total } }),
  };
};

export const useUpdateStore = create<UpdateStore>()(
  persist<UpdateStore, [], [], PersistedUpdatePrefs>(createUpdateStore, {
    name: UPDATE_PREFS_KEY,
    version: 1,
    storage: createJSONStorage<PersistedUpdatePrefs>(pickStorage),
    partialize: (s) => ({
      autoCheckEnabled: s.autoCheckEnabled,
      skippedVersion: s.skippedVersion,
    }),
    merge: mergeUpdatePrefs,
  }),
);
