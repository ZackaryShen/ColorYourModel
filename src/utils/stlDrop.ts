/** Routing decision for files dropped onto the window (issue #21).
 *
 * Drops share the 📂 import pipeline (`useImportStl`): one file with a
 * supported extension goes down exactly the same load path, everything else
 * gets a status-bar message instead of being silently swallowed by the
 * webview's drag-drop interception. A model that is already loaded is never
 * replaced without confirmation — a stray drop would otherwise discard
 * unexported painting (same stake as the exit confirm, issue #7).
 */

/** Extensions the import pipeline understands: .stl imports a model, .3mf
 *  opens a full project (its plates) — the same pair the 📂 dialog filter
 *  offers. */
export const DROPPABLE_EXTENSIONS = ["stl", "3mf"] as const;

export type StlDropDecision =
  | { action: "import"; path: string }
  | { action: "confirm-replace"; path: string }
  | { action: "reject"; reason: "multi" | "extension" };

/** Turn whatever the OS drag-drop bridge handed us into a usable file path.
 *
 * Most platforms deliver a plain path, but two shapes show up in practice and
 * both silently failed the extension test before: a `file://` URL (sometimes
 * percent-encoded, so a Chinese file name arrives as `%E6%A2%81...`). The
 * import command receives the normalized path, so this is the only place that
 * has to know about the difference.
 */
export function normalizeDroppedPath(raw: string): string {
  if (!/^file:\/\//i.test(raw)) return raw;
  let path = raw.slice("file://".length);
  {
    if (/^localhost\//i.test(path)) path = path.slice("localhost".length);
    // file:///C:/x -> /C:/x on Windows; drop the extra leading slash.
    if (/^\/[A-Za-z]:[\\/]/.test(path)) path = path.slice(1);
  }
  try {
    return decodeURIComponent(path);
  } catch {
    // Malformed escape sequence: keep the raw path rather than throwing.
    return path;
  }
}

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
  const path = normalizeDroppedPath(paths[0]);
  if (!hasDroppableExtension(path)) {
    return { action: "reject", reason: "extension" };
  }
  return hasModel ? { action: "confirm-replace", path } : { action: "import", path };
}
