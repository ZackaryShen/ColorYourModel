import { useState } from "react";
import { useAppStore } from "../store/appStore";
import { useTauriCommand } from "./useTauriCommand";
import { useT } from "../i18n";

/**
 * Shared merge flow for the ordered merge multi-selection (issue #16).
 *
 * Both entry points — SegmentsPanel rows and the Viewport's Ctrl+click plus
 * MergeBar — drive the same store `mergePicked` list through this hook, so the
 * anchor semantics (`picked[0]` is the surviving region) live in exactly one
 * place and the viewport highlight, panel chips and merge bar can never
 * disagree about what a merge will do.
 *
 * On success the selection collapses to the anchor (so the user sees what
 * remains and can keep merging into it); on rejection `mergePicked` is left
 * untouched and useTauriCommand has already surfaced the error.
 */
export function useSegmentMerge(): {
  picked: number[];
  merging: boolean;
  mergePicked: () => Promise<void>;
  clear: () => void;
} {
  const t = useT();
  const picked = useAppStore((s) => s.mergePicked);
  const setMergePicked = useAppStore((s) => s.setMergePicked);
  const setSelectedSegment = useAppStore((s) => s.setSelectedSegment);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const { mergeSegments } = useTauriCommand();
  const [merging, setMerging] = useState(false);

  const mergePicked = async (): Promise<void> => {
    const anchor = picked[0];
    // Guard re-entrant clicks and a bar/panel stale enough to hold <2 labels.
    if (anchor === undefined || picked.length < 2 || merging) return;
    setMerging(true);
    const moved = await mergeSegments(anchor, picked.slice(1));
    setMerging(false);
    if (moved === null) return;
    setStatusMessage(t("segments.merged", picked.length, moved));
    setMergePicked([anchor]);
    setSelectedSegment(anchor);
  };

  const clear = (): void => {
    useAppStore.getState().clearMergePicked();
  };

  return { picked, merging, mergePicked, clear };
}
