import type { HistoryEntryInfo } from "../types/mesh";

/** Row model for the history panel (issue #22).
 *
 * The backend's timeline is a list of incremental diffs plus a jump index:
 * depth k means the first k entries are applied (depth 0 = pristine). The
 * panel renders that as Photoshop does — a leading pristine row ("Open"),
 * one row per applied operation, a marker on the live state, and the redo
 * branch below it, greyed but jump-forward-able. A row's `depth` IS the
 * argument `history_jump` takes, so clicking a row is one IPC call.
 *
 * Pure: labels/icons/locale formatting live in the component; this module
 * only decides structure. Tested in historyPanel.test.ts.
 */

/** Same shape as the backend's `HistoryEntryDto` (commands/history.rs). */
export type HistoryPanelEntry = HistoryEntryInfo;

export interface HistoryRow {
  /** Jump target for `history_jump`: 0 = pristine, k = after op k. */
  depth: number;
  /** Wire key for the label; `null` on the pristine row. */
  kind: HistoryEntryInfo["kind"] | null;
  strokeId: number | null;
  faceCount: number | null;
  bytes: number;
  /** The live state's marker — exactly one row per fetch. */
  isCurrent: boolean;
  /** Below the marker: the redo branch, greyed, click jumps forward. */
  isRedo: boolean;
}

/** Pristine row + one row per applied entry, oldest first. `current` is the
 *  backend's live depth (`HistoryEntriesResult.current`). */
export function buildHistoryRows(entries: HistoryPanelEntry[], current: number): HistoryRow[] {
  const rows: HistoryRow[] = [
    // Pristine row: above the marker whenever anything is applied — never
    // on the redo branch, no matter how far back the user sits.
    { depth: 0, kind: null, strokeId: null, faceCount: null, bytes: 0, isCurrent: current === 0, isRedo: false },
  ];
  entries.forEach((e, i) => {
    const depth = i + 1;
    rows.push({
      depth,
      kind: e.kind,
      strokeId: e.strokeId,
      faceCount: e.faceCount,
      bytes: e.bytes,
      isCurrent: depth === current,
      isRedo: depth > current,
    });
  });
  return rows;
}

/** Header footprint, e.g. "3.4 MB" — matching the issue's `第 12 / 50 步 · 3.4 MB`. */
export function formatHeaderBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
