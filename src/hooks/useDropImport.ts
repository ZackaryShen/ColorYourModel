import { useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useAppStore } from "../store/appStore";
import { useT } from "../i18n";
import { planFileDrop } from "../utils/stlDrop";
import { log } from "../utils/logger";

/**
 * Drag-and-drop STL/.cym import (issue #21).
 *
 * With the default `dragDropEnabled: true` the webview intercepts OS
 * drag-drop on Windows, so HTML5 dragover/drop handlers never fire — the
 * events must come from `getCurrentWebview().onDragDropEvent()`. The decision
 * itself lives in the pure `planFileDrop`; this hook only owns the Tauri
 * wiring: overlay visibility, the replace confirmation state, and routing
 * accepted drops into the SHARED import pipeline (`importFromPath`), so the
 * 📂 button and a drop can never drift apart.
 */
export function useDropImport(importFromPath: (path: string) => Promise<void>) {
  const t = useT();
  const [dragOver, setDragOver] = useState(false);
  const [pendingReplacePath, setPendingReplacePath] = useState<string | null>(null);

  // Live refs so the once-registered listener always reads the latest closure
  // without resubscribing on every render (same pattern as handleFacePickedRef
  // in Viewport — `t` is re-created each render, language is not).
  const importFromPathRef = useRef(importFromPath);
  importFromPathRef.current = importFromPath;
  const tRef = useRef(t);
  tRef.current = t;

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "enter") {
          setDragOver(true);
          return;
        }
        // "over" fires continuously while hovering; the overlay just stays.
        if (payload.type === "leave") {
          setDragOver(false);
          return;
        }
        if (payload.type !== "drop") return;
        setDragOver(false);
        const setStatusMessage = useAppStore.getState().setStatusMessage;

        // An import in flight owns the screen (progress bar); a drop during
        // it has no sensible target. Rejected, but never silently — issue #21
        // requires a message for every swallowed drop.
        if (useAppStore.getState().isLoading) {
          setStatusMessage(tRef.current("drop.importInProgress"));
          return;
        }

        const decision = planFileDrop(
          payload.paths,
          useAppStore.getState().meshData !== null
        );
        log.info("dropImport", "drop routed", { paths: payload.paths, decision });
        if (decision.action === "import") {
          void importFromPathRef.current(decision.path);
        } else if (decision.action === "confirm-replace") {
          setPendingReplacePath(decision.path);
        } else if (decision.reason === "multi") {
          setStatusMessage(tRef.current("drop.rejectMulti"));
        } else {
          setStatusMessage(tRef.current("drop.rejectExtension"));
        }
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const confirmReplace = () => {
    const path = pendingReplacePath;
    setPendingReplacePath(null);
    if (path) void importFromPathRef.current(path);
  };

  const cancelReplace = () => setPendingReplacePath(null);

  return { dragOver, pendingReplacePath, confirmReplace, cancelReplace };
}
