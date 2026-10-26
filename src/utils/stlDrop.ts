/** Routing decision for files dropped onto the window (issue #21).
 *
 * Drops share the 📂 import pipeline (`useImportStl`): one file with a
 * supported extension goes down exactly the same load path, everything else
 * gets a status-bar message instead of being silently swallowed by the
 * webview's drag-drop interception. A model that is already loaded is never
 * replaced without confirmation — a stray drop would otherwise discard
 * unexported painting (same stake as the exit confirm, issue #7).
 */

/** Extensions the import pipeline understands: .stl imports a model, .cym
 *  opens a full project — the same pair the 📂 dialog filter offers. */
export const DROPPABLE_EXTENSIONS = ["stl", "cym"] as const;

export type StlDropDecision =
  | { action: "import"; path: string }
  | { action: "confirm-replace"; path: string }
  | { action: "reject"; reason: "multi" | "extension" };

function hasDroppableExtension(path: string): boolean {
  const lower = path.toLowerCase();
  return DROPPABLE_EXTENSIONS.some((ext) => lower.endsWith(`.${ext}`));
}

/** Decide what a drop of `paths` should do. Pure: the caller owns all I/O.
 *  `hasModel` reflects whether a model is already loaded (replacement risk). */
export function planFileDrop(paths: string[], hasModel: boolean): StlDropDecision {
  if (paths.length !== 1) {
    return { action: "reject", reason: "multi" };
  }
  const path = paths[0];
  if (!hasDroppableExtension(path)) {
    return { action: "reject", reason: "extension" };
  }
  return hasModel ? { action: "confirm-replace", path } : { action: "import", path };
}
