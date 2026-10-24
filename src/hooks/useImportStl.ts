import { open } from "@tauri-apps/plugin-dialog";
import { useT } from "../i18n";
import { useAppStore } from "../store/appStore";
import { useTauriCommand } from "./useTauriCommand";
import { DROPPABLE_EXTENSIONS } from "../utils/stlDrop";
import { log } from "../utils/logger";

/**
 * Shared STL import flow (file picker → import progress → loadModel → status
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
  const { loadModel, loadProject } = useTauriCommand();
  const setLoading = useAppStore((s) => s.setLoading);
  const setImportProgress = useAppStore((s) => s.setImportProgress);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);

  const importFromPath = async (selected: string) => {
    log.info("importStl", `Importing: ${selected}`);
    setLoading(true);
    setImportProgress(0, t("toolbar.startImport"));

    try {
      if (selected.toLowerCase().endsWith(".cym")) {
        await loadProject(selected);
      } else {
        await loadModel(selected);
      }
    } catch (e) {
      log.error("importStl", "Import failed", { error: String(e) });
      setStatusMessage(`${t("toolbar.importFailed")}: ${e}`);
      setLoading(false);
      return;
    }

    // No automatic segmentation on import (removed 2026-08-29): the fuse flow
    // (SeedPanel → 融合生成) builds its own vote channels and wipes existing
    // labels, so per-import compute was dead work. Region generation starts
    // explicitly from the Seed panel.
    setImportProgress(1, t("toolbar.importComplete"));
    setStatusMessage(t("toolbar.importComplete"));
    setLoading(false);
  };

  const importStl = async () => {
    log.info("importStl", "Import action triggered");
    const selected = await open({
      multiple: false,
      // 0.2.0-P1: the entry point is format-aware — .stl imports fresh, .cym
      // opens a full project (labels + paint + region names restored). The
      // filter and the drop gate (planFileDrop) share one extension list so
      // the two entry points can never drift apart.
      filters: [
        { name: "3D Models / CYM Project", extensions: [...DROPPABLE_EXTENSIONS] },
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
