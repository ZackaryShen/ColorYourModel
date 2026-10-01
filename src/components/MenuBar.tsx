import { useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useAppStore } from "../store/appStore";
import { useImportStl } from "../hooks/useImportStl";
import { useT } from "../i18n";
import { log } from "../utils/logger";
import { ExportDialog } from "./ExportDialog/ExportDialog";

/** GitHub Pages site (docs/ tree, deployed by deploy-docs.yml). */
const WIKI_URL = "https://zackaryshen.github.io/ColorYourModel/";
const REPO_URL = "https://github.com/ZackaryShen/ColorYourModel";

type MenuId = "file" | "view" | "help";

/**
 * Classic top menu bar (File / View / Help) — the conventional entry points
 * (import/export, language & theme, docs, about, quit) that desktop apps are
 * expected to have, in addition to the left tool Toolbar. Quit goes through
 * getCurrentWindow().close() so the paint-dirty exit confirmation (issue #7)
 * applies to menu-triggered exits too.
 */
export function MenuBar() {
  const t = useT();
  const [openMenu, setOpenMenu] = useState<MenuId | null>(null);
  const [exportDialogOpen, setExportDialogOpen] = useState(false);
  const [aboutOpen, setAboutOpen] = useState(false);
  const importStl = useImportStl();

  const language = useAppStore((s) => s.language);
  const setLanguage = useAppStore((s) => s.setLanguage);
  const theme = useAppStore((s) => s.theme);
  const setTheme = useAppStore((s) => s.setTheme);
  const debugLogOpen = useAppStore((s) => s.debugLogOpen);
  const setDebugLogOpen = useAppStore((s) => s.setDebugLogOpen);

  const barRef = useRef<HTMLDivElement>(null);

  // Close on outside-click and Esc — the standard dismiss affordances for a
  // menu bar (inside clicks are stopPropagation'd by the dropdowns).
  useEffect(() => {
    if (!openMenu) return;
    const onDown = (e: MouseEvent) => {
      if (barRef.current && !barRef.current.contains(e.target as Node)) {
        setOpenMenu(null);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpenMenu(null);
    };
    document.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [openMenu]);

  const act = (fn: () => void) => () => {
    setOpenMenu(null);
    fn();
  };
  const openExternal = (url: string) => {
    openUrl(url).catch((e) => log.error("MenuBar", "openUrl failed", { url, error: String(e) }));
  };

  const menus: {
    id: MenuId;
    label: string;
    items: (
      | { kind: "item"; label: string; danger?: boolean; onClick: () => void }
      | { kind: "sep" }
    )[];
  }[] = [
    {
      id: "file",
      label: t("menu.file"),
      items: [
        { kind: "item", label: t("menu.import"), onClick: () => importStl() },
        { kind: "item", label: t("menu.export"), onClick: () => setExportDialogOpen(true) },
        { kind: "sep" },
        // close() re-enters the onCloseRequested handler, so unexported work
        // still gets the exit confirmation; a clean state just closes.
        { kind: "item", label: t("menu.quit"), danger: true, onClick: () => getCurrentWindow().close() },
      ],
    },
    {
      id: "view",
      label: t("menu.view"),
      items: [
        { kind: "item", label: t("menu.language"), onClick: () => setLanguage(language === "zh" ? "en" : "zh") },
        { kind: "item", label: theme === "dark" ? t("menu.themeToLight") : t("menu.themeToDark"), onClick: () => setTheme(theme === "dark" ? "light" : "dark") },
        { kind: "sep" },
        { kind: "item", label: t("menu.debugLog"), onClick: () => setDebugLogOpen(!debugLogOpen) },
      ],
    },
    {
      id: "help",
      label: t("menu.help"),
      items: [
        { kind: "item", label: t("menu.wiki"), onClick: () => openExternal(WIKI_URL) },
        { kind: "sep" },
        { kind: "item", label: t("menu.about"), onClick: () => setAboutOpen(true) },
      ],
    },
  ];

  return (
    <div ref={barRef} style={styles.bar}>
      <span style={styles.brand}>ColorYourModel</span>
      {menus.map((m) => (
        <div key={m.id} style={styles.menuWrap}>
          <button
            className="cym-btn"
            style={{
              ...styles.top,
              ...(openMenu === m.id ? styles.topActive : {}),
            }}
            onClick={() => setOpenMenu(openMenu === m.id ? null : m.id)}
            aria-haspopup="menu"
            aria-expanded={openMenu === m.id}
          >
            {m.label}
          </button>
          {openMenu === m.id && (
            <div style={styles.dropdown} role="menu">
              {m.items.map((item, i) =>
                item.kind === "sep" ? (
                  <div key={i} style={styles.sep} />
                ) : (
                  <button
                    key={i}
                    className="cym-btn"
                    style={{ ...styles.item, ...(item.danger ? styles.itemDanger : {}) }}
                    role="menuitem"
                    onClick={act(item.onClick)}
                  >
                    {item.label}
                  </button>
                )
              )}
            </div>
          )}
        </div>
      ))}

      {exportDialogOpen && <ExportDialog onClose={() => setExportDialogOpen(false)} />}
      {aboutOpen && <AboutDialog onClose={() => setAboutOpen(false)} />}
    </div>
  );
}

/// Conventional About box: name/version/stack/licence/repo, same overlay
/// language as the other dialogs. Esc and overlay-click both close.
function AboutDialog({ onClose }: { onClose: () => void }) {
  const t = useT();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const row = (k: string, v: string) => (
    <div style={styles.aboutRow}>
      <span style={styles.aboutKey}>{k}</span>
      <span style={styles.aboutVal}>{v}</span>
    </div>
  );

  return (
    <div style={styles.overlay} onClick={onClose}>
      <div style={styles.dialog} onClick={(e) => e.stopPropagation()} role="dialog" aria-modal="true">
        <div style={styles.aboutTitleRow}>
          <span style={{ fontSize: 22 }} aria-hidden>
            🎨
          </span>
          <span style={styles.aboutTitle}>ColorYourModel</span>
        </div>
        <div style={styles.aboutDesc}>{t("about.desc")}</div>
        <div style={{ marginTop: 10 }}>
          {row(t("about.version"), `v${__APP_VERSION__}`)}
          {row(t("about.tech"), "Tauri 2 · React 18 · Rust")}
          {row(t("about.license"), "AGPL-3.0")}
          {row(t("about.repo"), REPO_URL)}
        </div>
        <div style={styles.btnRow}>
          <button className="cym-btn" style={styles.btnPrimary} onClick={onClose}>
            {t("about.close")}
          </button>
        </div>
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  bar: {
    display: "flex",
    alignItems: "center",
    gap: 2,
    padding: "2px 8px",
    background: "var(--bg-elevated, #252525)",
    borderBottom: "1px solid var(--border-strong, #333333)",
    color: "var(--text-2, #aaaaaa)",
    fontSize: 12,
    userSelect: "none",
    zIndex: 100,
  },
  brand: {
    fontWeight: 700,
    color: "var(--text-1, #eeeeee)",
    marginRight: 10,
    fontSize: 12,
  },
  menuWrap: {
    position: "relative",
  },
  top: {
    padding: "3px 10px",
    border: "none",
    background: "transparent",
    color: "var(--text-2, #aaaaaa)",
    borderRadius: 4,
    cursor: "pointer",
    fontSize: 12,
  },
  topActive: {
    background: "var(--bg-active, #3a5a7a)",
    color: "var(--text-1, #eeeeee)",
  },
  dropdown: {
    position: "absolute",
    top: "100%",
    left: 0,
    minWidth: 200,
    marginTop: 2,
    padding: 4,
    background: "var(--bg-panel, #2d2d2d)",
    border: "1px solid var(--border, #555)",
    borderRadius: 8,
    boxShadow: "0 8px 32px rgba(0,0,0,0.45)",
    display: "flex",
    flexDirection: "column",
    zIndex: 1000,
  },
  item: {
    display: "block",
    width: "100%",
    textAlign: "left",
    padding: "6px 10px",
    border: "none",
    background: "transparent",
    color: "var(--text-1, #eeeeee)",
    borderRadius: 4,
    cursor: "pointer",
    fontSize: 12.5,
    whiteSpace: "nowrap",
  },
  itemDanger: {
    color: "var(--danger, #e5484d)",
  },
  sep: {
    height: 1,
    margin: "4px 8px",
    background: "var(--border, #555)",
  },
  overlay: {
    position: "fixed",
    inset: 0,
    background: "rgba(0,0,0,0.55)",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    zIndex: 1100,
  },
  dialog: {
    width: 400,
    background: "var(--bg-panel, #2d2d2d)",
    border: "1px solid var(--border, #555)",
    borderRadius: 10,
    padding: 18,
    color: "var(--text-1, #eee)",
    fontFamily: "inherit",
    boxShadow: "0 8px 32px rgba(0,0,0,0.45)",
  },
  aboutTitleRow: {
    display: "flex",
    alignItems: "center",
    gap: 8,
  },
  aboutTitle: {
    fontSize: 16,
    fontWeight: 700,
  },
  aboutDesc: {
    marginTop: 8,
    fontSize: 12.5,
    color: "var(--text-2, #aaa)",
    lineHeight: 1.6,
  },
  aboutRow: {
    display: "flex",
    gap: 8,
    fontSize: 12,
    lineHeight: 1.8,
  },
  aboutKey: {
    width: 110,
    flexShrink: 0,
    color: "var(--text-3, #888)",
  },
  aboutVal: {
    color: "var(--text-1, #eee)",
    wordBreak: "break-all",
  },
  btnRow: {
    display: "flex",
    justifyContent: "flex-end",
    gap: 8,
    marginTop: 14,
  },
  btnPrimary: {
    padding: "6px 14px",
    border: "1px solid var(--accent-border, #2b6cb0)",
    background: "var(--accent, #3a5a7a)",
    color: "#fff",
    borderRadius: 6,
    cursor: "pointer",
    fontSize: 13,
    fontWeight: 600,
  },
};
