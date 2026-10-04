import { useEffect } from "react";
import { useAppStore } from "../store/appStore";
import { shouldAutoCheckNow, useUpdateStore } from "../store/updateStore";
import { log } from "../utils/logger";

/** Startup delay before the silent update check. Long enough that the check
 *  never competes with the window's first paint or the module-import burst;
 *  short enough that users who only glance at the app still see a prompt. */
const AUTO_CHECK_DELAY_MS = 4_000;

/** When the app is busy at fire time (import / fuse / lasso finalize all set
 *  isLoading), retry later instead of throwing a dialog over active work.
 *  5 tries x 30s ≈ 2.5 minutes of patience; after that we silently give up
 *  until the next launch (one check per session, REFUTE M5). */
const BUSY_RETRY_MS = 30_000;
const BUSY_RETRY_LIMIT = 5;

/**
 * Silent startup update check — mounted ONCE in App. Policy (updateStore):
 * dev builds and opted-out users never check; one check per session; busy
 * startup defers; a skipped version stays silent; every failure is silent.
 */
export function useAutoUpdateCheck() {
  useEffect(() => {
    // Dev exes run an arbitrary version line unrelated to published releases —
    // an auto prompt there is definitionally a false alarm. (REFUTE M-误提醒 4.)
    if (import.meta.env.DEV) return;

    let retries = 0;
    let timer: number | undefined;

    const tryCheck = () => {
      const { autoCheckEnabled, phase } = useUpdateStore.getState();
      if (!shouldAutoCheckNow({ isDev: false, autoCheckEnabled })) return;
      // If the user opened the updater themselves, never fight its state machine.
      if (phase !== "idle") return;
      // Import / fuse / lasso-finalize all spin isLoading; prompting over them
      // is exactly the interruption the plan promised to avoid.
      if (useAppStore.getState().isLoading) {
        retries += 1;
        if (retries <= BUSY_RETRY_LIMIT) {
          log.info("update", "startup busy — deferring update check", { retries });
          timer = window.setTimeout(tryCheck, BUSY_RETRY_MS);
        }
        return;
      }
      useUpdateStore.getState().startAutoCheck();
    };

    timer = window.setTimeout(tryCheck, AUTO_CHECK_DELAY_MS);
    return () => {
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, []);
}
