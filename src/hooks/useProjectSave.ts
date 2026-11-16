import { save } from "@tauri-apps/plugin-dialog";
import { useT } from "../i18n";
import { useAppStore } from "../store/appStore";
import { useTauriCommand } from "./useTauriCommand";
import { log } from "../utils/logger";

/**
 * Save the current model back into a 3MF.
 *
 * A 3MF opened through the import pipeline is both the model and its project
 * file (it carries the engineering parts), so "save" writes the same export
 * the dialog would produce — including the machine/process/filament selection
 * the user last used — straight into the file they opened. "Save as" asks for a
 * new path and becomes the remembered one. Models opened from an STL have no
 * project path, so the first save behaves like "save as".
 */
export function useProjectSave() {
  const t = useT();
  const { export3mf } = useTauriCommand();
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const setProjectPath = useAppStore((s) => s.setProjectPath);

  const write = async (saveAs: boolean) => {
    const state = useAppStore.getState();
    if (!state.meshData) return;

    let path = state.projectPath;
    if (saveAs || !path) {
      const target = await save({
        filters: [{ name: "3MF Project", extensions: ["3mf"] }],
        defaultPath: path ?? "project.3mf",
      });
      if (!target) return;
      // The save dialog can return the bare name the user typed.
      path = target.toLowerCase().endsWith(".3mf") ? target : `${target}.3mf`;
    }

    try {
      await export3mf(path, state.lastExportSelection ?? undefined);
      setProjectPath(path);
      setStatusMessage(`${t("status.projectSaved")}: ${path}`);
    } catch (e) {
      log.error("projectSave", "save failed", { path, error: String(e) });
      setStatusMessage(`${t("toolbar.saveProjectFailed")}: ${e}`);
    }
  };

  return {
    saveProject: () => write(false),
    saveProjectAs: () => write(true),
  };
}
