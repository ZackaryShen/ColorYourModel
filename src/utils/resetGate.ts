// ── Issue #28: the destructive-reset confirm gate ──────────────────
// `window.confirm` in the Tauri WebView is intercepted by
// tauri-plugin-dialog and resolves ASYNC (and `plugin:dialog|confirm`
// is not even in the capability ACL, so it rejects outright). The old
// sync guard `if (!window.confirm(...)) return` therefore saw a truthy
// Promise and never bailed: 清空分区/重置分区 wiped the model unasked.
// The replacement is an in-app dialog whose lifecycle this state
// machine pins: the destructive action is reachable only through an
// open dialog, fires at most once per confirmation, and cannot be
// cancelled once the backend reset is in flight.

/** Where a destructive reset currently stands. */
export type ResetPhase = "idle" | "confirming" | "running";

/** What the user (or the reset itself) just did. */
export type ResetEvent = "request" | "confirm" | "cancel" | "settled";

/**
 * Pure transition table, driven from the panels with
 * `setPhase((p) => resetGateTransition(p, event))`.
 * Unknown/stale events are no-ops that return the current phase, so a
 * late click can never push the flow off the happy path
 * idle → confirming → running → idle.
 */
export function resetGateTransition(phase: ResetPhase, event: ResetEvent): ResetPhase {
  switch (event) {
    case "request":
      return phase === "idle" ? "confirming" : phase;
    case "confirm":
      return phase === "confirming" ? "running" : phase;
    case "cancel":
      return phase === "confirming" ? "idle" : phase;
    case "settled":
      return phase === "running" ? "idle" : phase;
  }
}
