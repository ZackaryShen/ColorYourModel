import { open, save } from "@tauri-apps/plugin-dialog";
import { useT } from "../i18n";
import { useAppStore } from "../store/appStore";
import { useTauriCommand } from "./useTauriCommand";
import { log } from "../utils/logger";

/**
 * .cym project flows (0.2.0-P1): open (dialog → load_project → meshData
 * restore) and save-as (dialog → save_project → paint marked clean).
 *
 * v1 limitation (documented in the 0.2.0 plan): every save asks for a path —
 * there is no Ctrl+S / last-path memory yet.
 */
export function useProject() {
  const t = useT();
  const { loadProject, saveProject } = useTauriCommand();
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);

  const openProject = async () => {
    log.info("openProject", "Open project action triggered");
    const selected = await open({
      multiple: false,
      filters: [{ name: "CYM Project", extensions: ["cym"] }],
    });
    if (!selected) {
      log.debug("openProject", "File dialog cancelled");
      return;
    }
    log.info("openProject", `Project selected: ${selected}`);
    try {
      await loadProject(selected);
    } catch (e) {
      log.error("openProject", "Open project failed", { error: String(e) });
      setStatusMessage(`${t("toolbar.openProjectFailed")}: ${e}`);
    }
  };

  const saveProjectAs = async () => {
    log.info("saveProjectAs", "Save project action triggered");
    const selected = await save({
      filters: [{ name: "CYM Project", extensions: ["cym"] }],
      defaultPath: "project.cym",
    });
    if (!selected) {
      log.debug("saveProjectAs", "File dialog cancelled");
      return;
    }
    // Mirror ExportDialog's double-extension guard (the save dialog can
    // return the bare name when the user types it without the extension).
    const path = selected.toLowerCase().endsWith(".cym") ? selected : `${selected}.cym`;
    log.info("saveProjectAs", `Saving project to ${path}`);
    try {
      await saveProject(path);
      setStatusMessage(t("status.projectSaved"));
    } catch (e) {
      log.error("saveProjectAs", "Save project failed", { error: String(e) });
      setStatusMessage(`${t("toolbar.saveProjectFailed")}: ${e}`);
    }
  };

  return { openProject, saveProjectAs };
}
