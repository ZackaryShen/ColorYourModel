import { describe, it, expect } from "vitest";
import { resetGateTransition, type ResetPhase, type ResetEvent } from "./resetGate";

// ── Issue #28: the destructive-reset confirm gate ──────────────────
// `window.confirm` in the Tauri WebView is intercepted by the dialog
// plugin and resolves ASYNC: the sync guard `if (!window.confirm(...))`
// sees a truthy Promise, never bails, and the reset ran unasked. The
// replacement flow is an in-app dialog driven by this tiny state
// machine, so the transition table below IS the contract:
//   - the destructive action (confirm → running) is reachable ONLY
//     through an open dialog (confirming), never straight from idle;
//   - it fires at most once per confirmed dialog (running ignores
//     further confirms — no double reset_segmentation IPC);
//   - an in-flight reset cannot be cancelled out of (cancel while
//     running is a no-op — the backend is already wiping state).
describe("resetGateTransition — issue #28 confirm gate", () => {
  const table: [ResetPhase, ResetEvent, ResetPhase][] = [
    // happy path: idle → dialog open → confirmed → settled
    ["idle", "request", "confirming"],
    ["confirming", "confirm", "running"],
    ["running", "settled", "idle"],
    // user cancels the dialog — the exact path #28 lost
    ["confirming", "cancel", "idle"],
    // the invariants #28 is about
    ["idle", "confirm", "idle"], // no dialog → no destructive action, ever
    ["idle", "cancel", "idle"],
    ["idle", "settled", "idle"],
    ["confirming", "request", "confirming"], // dialog already open
    ["confirming", "settled", "confirming"],
    ["running", "confirm", "running"], // no double-fire (double-click guard)
    ["running", "cancel", "running"], // cannot cancel an in-flight wipe
    ["running", "request", "running"],
  ];

  it.each(table)("%s + %s → %s", (phase, event, expected) => {
    expect(resetGateTransition(phase, event)).toBe(expected);
  });
});
