import { open } from "@tauri-apps/plugin-dialog";
import { useT } from "../i18n";
import { useAppStore } from "../store/appStore";
import { useTauriCommand } from "./useTauriCommand";
import { log } from "../utils/logger";

/**
 * Shared STL import flow (file picker → import progress → loadModel → status
 * messages). Extracted from the Toolbar button (iteration 14) when the File
 * menu needed the same behaviour; the Toolbar keeps its own progress-event
 * listeners, this hook only owns the click-to-loaded path.
 */
export function useImportStl() {
  const t = useT();
  const { loadModel, loadProject } = useTauriCommand();
  const setLoading = useAppStore((s) => s.setLoading);
  const setImportProgress = useAppStore((s) => s.setImportProgress);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);

  const importStl = async () => {
    log.info("importStl", "Import action triggered");
    const selected = await open({
      multiple: false,
      // 0.2.0-P1: the entry point is format-aware — .stl imports fresh, .cym
      // opens a full project (labels + paint + region names restored).
      filters: [{ name: "3D Models / CYM Project", extensions: ["stl", "cym"] }],
    });
    if (!selected) {
      log.debug("importStl", "File dialog cancelled");
      return;
    }

    log.info("importStl", `File selected: ${selected}`);
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

  return importStl;
}
