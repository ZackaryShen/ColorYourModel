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
  "plateIndex" | "plateCount" | "engineeringState" | "historyDropped" | "importWarning"
>;

/**
 * Status-line text after an import.
 *
 * Keep state-loss warnings and the partial multi-plate import notice together:
 * either limitation can affect whether saving the opened model is safe.
 */
export function importStatusMessage(
  data: ImportStatusFields,
  t: (key: string, ...args: (string | number)[]) => string
): string {
  const warnings: string[] = [];
  if (data.engineeringState === "dropped") warnings.push(t("status.engineeringDropped"));
  else if (data.historyDropped) warnings.push(t("status.historyDropped"));
  if (data.importWarning === "3mf-texture") warnings.push(t("project.openWarning.3mf-texture"));
  // The DTO fields are additive; absent means a single-plate source (STL or an
  // older payload).
  const plateIndex = data.plateIndex ?? 1;
  const plateCount = data.plateCount ?? 1;
  if (plateCount > 1) warnings.push(t("toolbar.importPlateOfTotal", plateIndex, plateCount));
  return warnings.length > 0 ? warnings.join(" · ") : t("toolbar.importComplete");
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
export function useImportStl(openTexture?: (path: string) => void) {
  const t = useT();
  const { loadModel } = useTauriCommand();
  const setLoading = useAppStore((s) => s.setLoading);
  const setImportProgress = useAppStore((s) => s.setImportProgress);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const setImportError = useAppStore((s) => s.setImportError);

  const pendingReplacePath = useAppStore((s) => s.pendingImportPath);
  const blocked = () => {
    const state = useAppStore.getState();
    if (!state.isLoading && !state.importBusy && !state.saveBusy && !state.pendingImportPath && state.pendingCloseTabId == null) return false;
    setStatusMessage(t("drop.importInProgress"));
    return true;
  };

  const performImport = async (selected: string) => {
    if (blocked()) return;
    useAppStore.setState({ importBusy: true });
    log.info("importStl", `Importing: ${selected}`);
    useAppStore.setState({loadingKind: "import", importProgress: 0, importStage: t("toolbar.startImport")});
    setLoading(true);

    try {
      const data = await loadModel(selected);
      if (/\.3mf$/i.test(selected)) {
        const restored = data.engineeringState === "restored";
        useAppStore.setState({
          projectPath: restored ? selected : null,
          importSourcePath: restored ? null : selected,
          lastExportSelection: null,
        });
      }
      // No automatic segmentation on import (removed 2026-08-29): the fuse flow
      // (SeedPanel → 融合生成) builds its own vote channels and wipes existing
      // labels, so per-import compute was dead work. Region generation starts
      // explicitly from the Seed panel.
      setImportProgress(1, t("toolbar.importComplete"));
      setStatusMessage(importStatusMessage(data, t));
    } catch (e) {
      log.error("importStl", "Import failed", { error: String(e) });
      // P0 2026-10-12: a status-bar line alone let a broken/unsupported file
      // pass almost unnoticed — surface a modal as well (raw backend string).
      setImportError(String(e));
      setStatusMessage(`${t("toolbar.importFailed")}: ${e}`);
    } finally {
      setLoading(false);
      useAppStore.setState({ importBusy: false });
    }
  };

  const importFromPath = async (selected: string, confirmLoaded = false) => {
    if (blocked()) return;
    if (/\.glb$/i.test(selected) && openTexture) {
      openTexture(selected);
      return;
    }
    const state = useAppStore.getState();
    if (/\.3mf$/i.test(selected) && state.meshData && (state.paintDirty || state.tabs.some(tab => tab.dirty) || confirmLoaded)) {
      useAppStore.setState({ pendingImportPath: selected });
      return;
    }
    await performImport(selected);
  };

  const cancelReplace = () => useAppStore.setState({ pendingImportPath: null });
  const confirmReplace = async () => {
    const path = useAppStore.getState().pendingImportPath;
    if (!path) return;
    cancelReplace();
    await performImport(path);
  };

  const importStl = async () => {
    if (blocked()) return;
    useAppStore.setState({ importBusy: true });
    log.info("importStl", "Import action triggered");
    let selected: string | null;
    try {
      selected = await open({
        multiple: false,
        // Picker and drop routing accept the same formats.
        filters: [
          { name: t("menu.modelFileFilter"), extensions: [...DROPPABLE_EXTENSIONS] },
        ],
      });
    } catch (e) {
      setImportError(String(e));
      setStatusMessage(`${t("toolbar.importFailed")}: ${e}`);
      return;
    } finally {
      useAppStore.setState({ importBusy: false });
    }
    if (!selected) {
      log.debug("importStl", "File dialog cancelled");
      return;
    }

    await importFromPath(selected);
  };

  return { importStl, importFromPath, pendingReplacePath, confirmReplace, cancelReplace };
}
