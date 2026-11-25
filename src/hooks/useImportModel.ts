import { useRef } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useT } from "../i18n";
import { useAppStore } from "../store/appStore";
import { useTauriCommand } from "./useTauriCommand";
import { DROPPABLE_EXTENSIONS, isTextureImport } from "../utils/stlDrop";
import { log } from "../utils/logger";

/** File picker and drag-drop share the same format-specific import paths. */
export function useImportModel(textureImport: { open: (path: string) => void; isOpen: () => boolean }) {
  const pickerOpen = useRef(false);
  const t = useT();
  const { loadModel } = useTauriCommand();
  const setLoading = useAppStore((s) => s.setLoading);
  const setImportProgress = useAppStore((s) => s.setImportProgress);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const setImportError = useAppStore((s) => s.setImportError);

  const isImportBusy = () =>
    pickerOpen.current || textureImport.isOpen() || useAppStore.getState().isLoading;

  const importFromPath = async (selected: string) => {
    if (isImportBusy()) {
      setStatusMessage(t("drop.importInProgress"));
      return;
    }
    if (isTextureImport(selected)) {
      textureImport.open(selected);
      return;
    }
    log.info("importModel", `Importing: ${selected}`);
    useAppStore.getState().setLoadingKind("import");
    setLoading(true);
    setImportProgress(0, t("toolbar.startImport"));

    try {
      const data = await loadModel(selected);
      if (data.importWarning) {
        const key = "project.openWarning." + data.importWarning;
        useAppStore.getState().setToast(t(key));
      }
    } catch (e) {
      log.error("importModel", "Import failed", { error: String(e) });
      // P0 2026-10-12: a status-bar line alone let a broken/unsupported file
      // pass almost unnoticed — surface a modal as well (raw backend string).
      setImportError(String(e));
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

  const pickModel = async () => {
    if (isImportBusy()) return;
    pickerOpen.current = true;
    let selected: string | null = null;
    try {
      selected = await open({
        multiple: false,
        filters: [
          { name: t("menu.modelFileFilter"), extensions: [...DROPPABLE_EXTENSIONS] },
        ],
      });
    } catch (error) {
      log.error("importModel", "File picker failed", { error: String(error) });
      setImportError(String(error));
    } finally {
      pickerOpen.current = false;
    }
    if (selected) await importFromPath(selected);
  };

  return { pickModel, importFromPath, isImportBusy };
}
