/**
 * Lightweight i18n system — English fallback, Chinese in Chinese locales
 * (default follows the OS locale; see appStore).
 * Usage: const t = useT();  then  t("toolbar.import")
 *
 * The dictionaries and the pure translate()/translateError() helpers live in
 * i18nDict.ts (dependency-free) so non-React modules (the Zustand store,
 * event handlers) can localise without an import cycle; this file only adds
 * the React hook wrapper.
 */
import { useAppStore } from "./store/appStore";
import { translate, translateError, type Lang } from "./i18nDict";

export { translate, translateError };
export type { Lang };

export function useT() {
  const lang = useAppStore((s) => s.language);
  return (key: string, ...args: (string | number)[]): string =>
    translate(key, lang, ...args);
}
