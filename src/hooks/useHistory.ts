import { useCallback } from "react";
import { useAppStore } from "../store/appStore";
import { log } from "../utils/logger";
import type { HistoryResult, HistoryState } from "../types/mesh";

/**
 * Bridge between the backend-owned undo/redo timeline and the React tree.
 *
 * The live `updateFaceColors` (and therefore the GPU color buffer) lives inside
 * the single `useMesh` instance that `MeshDisplay` mounts. The toolbar buttons
 * and the global keyboard handler also need to apply a `HistoryResult`, but they
 * cannot hold that instance. So `MeshDisplay` registers `applyHistory` here once,
 * and anyone with a `HistoryResult` calls `applyHistoryResult` to run it through
 * the real mesh.
 */

type Applier = (r: HistoryResult | null) => void;

let applier: Applier | null = null;
// Guards the IPC round-trip so a held Ctrl key (auto-repeat keydown) or a
// double-click can't stack undo/redo commands against the backend.
let draining = false;

export function setHistoryApplier(fn: Applier | null) {
  applier = fn;
}

export function applyHistoryResult(r: HistoryResult | null) {
  if (applier) applier(r);
}

export interface UndoRedoCommands {
  undo: () => Promise<HistoryResult | null>;
  redo: () => Promise<HistoryResult | null>;
  historyState: () => Promise<HistoryState | null>;
}

/**
 * Exposes guarded `undo`/`redo` plus the live `canUndo`/`canRedo` flags.
 *
 * `cmds` is the set of raw IPC wrappers from `useTauriCommand`; passing them in
 * (rather than calling the hook internally) lets both `MeshDisplay` and the
 * `Toolbar` share one subscription without mounting a second `useMesh`.
 */
export function useUndoRedo(cmds: UndoRedoCommands) {
  const canUndo = useAppStore((s) => s.canUndo);
  const canRedo = useAppStore((s) => s.canRedo);

  const run = useCallback(
    async (fn: () => Promise<HistoryResult | null>) => {
      if (draining) return;
      draining = true;
      try {
        const res = await fn();
        applyHistoryResult(res);
      } catch (e) {
        log.error("useUndoRedo", "history operation failed", { error: String(e) });
      } finally {
        draining = false;
      }
    },
    [cmds.undo, cmds.redo]
  );

  const refresh = useCallback(async () => {
    const s = await cmds.historyState();
    if (s) useAppStore.getState().setHistoryFlags(s.canUndo, s.canRedo);
  }, [cmds.historyState]);

  return {
    canUndo,
    canRedo,
    undo: () => run(cmds.undo),
    redo: () => run(cmds.redo),
    refresh,
  };
}
