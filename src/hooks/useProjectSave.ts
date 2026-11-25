import { save } from "@tauri-apps/plugin-dialog";
import { useT } from "../i18n";
import { useAppStore } from "../store/appStore";
import { useTauriCommand } from "./useTauriCommand";
import { log } from "../utils/logger";

/** Save 3MF engineering state, retaining the imported project's presets.
 * The first save of an imported model chooses a project destination.
 */
let savingProject = false;

export function useProjectSave() {
  const t = useT();
  const { export3mf } = useTauriCommand();
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const setProjectPath = useAppStore((s) => s.setProjectPath);

  const write = async (saveAs: boolean) => {
    const state = useAppStore.getState();
    if (!state.meshData || state.isLoading || state.importBusy ||
        state.pendingImportPath || state.pendingCloseTabId != null || state.saveBusy || savingProject) return;
    savingProject = true;
    useAppStore.setState({ saveBusy: true });
    let path = state.projectPath;
    try {
      if (saveAs || !path) {
        const target = await save({
          filters: [{ name: "3MF Project", extensions: ["3mf"] }],
          defaultPath: path ?? state.importSourcePath?.replace(/\.3mf$/i, ".cym.3mf") ?? "project.3mf",
        });
        if (!target) return;
        path = target;
      }
      // A save dialog can stay open while an import replaces the current mesh.
      const current = useAppStore.getState();
      if (current.meshData !== state.meshData || current.isLoading ||
          current.importBusy || current.pendingImportPath || current.pendingCloseTabId != null) return;
      // Save the current project or the exact destination confirmed above.
      setStatusMessage(t("status.projectSaving"));
      const result = await export3mf(path, undefined, true);
      const actualPath = result.startsWith("Exported to ") ? result.slice(12) : path;
      setProjectPath(actualPath);
      setStatusMessage(`${t("status.projectSaved")}: ${actualPath}`);
    } catch (e) {
      log.error("projectSave", "save failed", { path, error: String(e) });
      setStatusMessage(`${t("toolbar.saveProjectFailed")}: ${e}`);
    } finally {
      savingProject = false;
      useAppStore.setState({ saveBusy: false });
    }
  };

  return {
    saveProject: () => write(false),
    saveProjectAs: () => write(true),
  };
}
