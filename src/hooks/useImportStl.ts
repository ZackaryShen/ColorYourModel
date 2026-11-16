import { open } from "@tauri-apps/plugin-dialog";
import { useT } from "../i18n";
import { useAppStore } from "../store/appStore";
import { useTauriCommand } from "./useTauriCommand";
import { DROPPABLE_EXTENSIONS } from "../utils/stlDrop";
import { log } from "../utils/logger";
import type { MeshData } from "../types/mesh";

/** The import report fields that decide the status-line message. */
export type ImportStatusFields = Pick<
  MeshData,
  "plateIndex" | "plateCount" | "engineeringState" | "historyDropped"
>;

/**
 * Status-line text after an import.
 *
 * Precedence matters: a dropped engineering state (saved labels/colours were
 * rejected, often because a slicer rewrote the file) is the most important
 * thing the user must know, then a dropped undo history, then the normal
 * multi-plate notice. STL and single-plate 3MF keep "import complete".
 */
export function importStatusMessage(
  data: ImportStatusFields,
  t: (key: string, ...args: (string | number)[]) => string
): string {
  if (data.engineeringState === "dropped") {
    return t("status.engineeringDropped");
  }
  if (data.historyDropped) {
    return t("status.historyDropped");
  }
  // The DTO fields are additive; absent means a single-plate source (STL or an
  // older payload).
  const plateIndex = data.plateIndex ?? 1;
  const plateCount = data.plateCount ?? 1;
  return plateCount > 1
    ? t("toolbar.importPlateOfTotal", plateIndex, plateCount)
    : t("toolbar.importComplete");
}

/**
 * Shared model import flow (file picker → import progress → loadModel → status
 * messages). Extracted from the Toolbar button (iteration 14) when the File
 * menu needed the same behaviour; the Toolbar keeps its own progress-event
 * listeners, this hook only owns the click-to-loaded path.
 *
 * `importFromPath` is the pipeline's single entry: the file dialog and the
 * drag-drop import (issue #21) both land here, so progress reporting and
 * error handling can never drift between the two entry points.
 */
export function useImportStl() {
  const t = useT();
  const { loadModel } = useTauriCommand();
  const setLoading = useAppStore((s) => s.setLoading);
  const setImportProgress = useAppStore((s) => s.setImportProgress);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const setImportError = useAppStore((s) => s.setImportError);
  const setProjectPath = useAppStore((s) => s.setProjectPath);

  const importFromPath = async (selected: string) => {
    log.info("importStl", `Importing: ${selected}`);
    setLoading(true);
    setImportProgress(0, t("toolbar.startImport"));

    try {
      const data = await loadModel(selected);
      setProjectPath(/\.3mf$/i.test(selected) ? selected : null);
      // No automatic segmentation on import (removed 2026-08-29): the fuse flow
      // (SeedPanel → 融合生成) builds its own vote channels and wipes existing
      // labels, so per-import compute was dead work. Region generation starts
      // explicitly from the Seed panel.
      setImportProgress(1, t("toolbar.importComplete"));
      setStatusMessage(importStatusMessage(data, t));
      setLoading(false);
    } catch (e) {
      log.error("importStl", "Import failed", { error: String(e) });
      // P0 2026-10-12: a status-bar line alone let a broken/unsupported file
      // pass almost unnoticed — surface a modal as well (raw backend string).
      setImportError(String(e));
      setStatusMessage(`${t("toolbar.importFailed")}: ${e}`);
      setLoading(false);
    }
  };

  const importStl = async () => {
    log.info("importStl", "Import action triggered");
    const selected = await open({
      multiple: false,
      // The entry point is format-aware — .3mf opens a full 3MF project (P0
      // loads its first plate), .stl imports a fresh model. The filter and the
      // drop gate (planFileDrop) share one extension list so the two entry
      // points can never drift apart.
      filters: [
        { name: "3MF Project / STL Model", extensions: [...DROPPABLE_EXTENSIONS] },
      ],
    });
    if (!selected) {
      log.debug("importStl", "File dialog cancelled");
      return;
    }

    await importFromPath(selected);
  };

  return { importStl, importFromPath };
}
