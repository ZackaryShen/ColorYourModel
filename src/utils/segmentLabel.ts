/** Display helpers for segment names.
 *
 * The backend mints default names as "Region N" (src-tauri/src/mesh/model.rs,
 * `format!("Region {}", ordinal)`) and stores them in the segment record, so
 * the strings travel through rename, merge and undo unchanged. That is the
 * right storage format — it is language-neutral and a user rename must win —
 * but it must not be what a Chinese UI renders.
 *
 * So the translation happens at DISPLAY time and only for names the user has
 * not touched: a name that still matches the backend's default pattern is
 * re-rendered through `segments.defaultName`. Anything else (a rename, a
 * future backend name, an empty string) is shown verbatim.
 */

const DEFAULT_NAME = /^Region (\d+)$/;

/** `t` is the translator from useT(); typed loosely so this module stays
 *  dependency-free and testable without React. */
type Translate = (key: string, ...args: (string | number)[]) => string;

/** Render a stored segment name for the current language. */
export function displaySegmentName(name: string, t: Translate): string {
  const m = DEFAULT_NAME.exec(name);
  if (!m) return name;
  return t("segments.defaultName", m[1]);
}

/** True when the name is still the backend's untouched default — used by the
 *  rename field so it starts from a localised, editable value. */
export function isDefaultSegmentName(name: string): boolean {
  return DEFAULT_NAME.test(name);
}
