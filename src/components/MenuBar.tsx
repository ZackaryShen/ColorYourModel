import { useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useAppStore } from "../store/appStore";
import { useUpdateStore } from "../store/updateStore";
import { useImportStl } from "../hooks/useImportStl";
import { useProject } from "../hooks/useProject";
import { useTauriCommand } from "../hooks/useTauriCommand";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { useT } from "../i18n";
import { log } from "../utils/logger";
import { ExportDialog } from "./ExportDialog/ExportDialog";
import { ProjectionDialog } from "./ProjectionDialog/ProjectionDialog";

/** GitHub Pages site (docs/ tree, deployed by deploy-docs.yml). The site root
 *  redirects to docs/README.html; the manuals and example gallery are the
 *  pages users actually need from the Help menu. */
const SITE_URL = "https://zackaryshen.github.io/ColorYourModel";
const MANUAL_URL: Record<"zh" | "en", string> = {
  zh: `${SITE_URL}/docs/user-guide/manual.html`,
  en: `${SITE_URL}/docs/user-guide/manual.en.html`,
};
const EXAMPLES_URL = `${SITE_URL}/docs/cases/examples.html`;
const REPO_URL = "https://github.com/ZackaryShen/ColorYourModel";
const DISCUSSIONS_URL = `${REPO_URL}/discussions`;

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
  const [projectionOpen, setProjectionOpen] = useState(false);
  const importStl = useImportStl();
  const { openProject, saveProjectAs } = useProject();
  const { exportSplitByColor } = useTauriCommand();

  const language = useAppStore((s) => s.language);
  const setLanguage = useAppStore((s) => s.setLanguage);
  const theme = useAppStore((s) => s.theme);
  const setTheme = useAppStore((s) => s.setTheme);
  const debugLogOpen = useAppStore((s) => s.debugLogOpen);
  const setDebugLogOpen = useAppStore((s) => s.setDebugLogOpen);
  // Mirrors the Toolbar export button's disabled={!isLoaded} guard: without a
  // mesh there are no presets/palette to pick in the ExportDialog.
  const hasMesh = useAppStore((s) => s.meshData !== null);

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
      | { kind: "item"; label: string; disabled?: boolean; hint?: string; danger?: boolean; onClick: () => void }
      | { kind: "sep" }
    )[];
  }[] = [
    {
      id: "file",
      label: t("menu.file"),
      items: [
        { kind: "item", label: t("menu.import"), onClick: () => importStl() },
        { kind: "item", label: t("menu.openProject"), onClick: () => void openProject() },
        {
          kind: "item",
          label: t("menu.saveProject"),
          disabled: !hasMesh,
          hint: hasMesh ? undefined : t("export.needModel"),
          onClick: () => void saveProjectAs(),
        },
        {
          kind: "item",
          label: t("menu.export"),
          disabled: !hasMesh,
          hint: hasMesh ? undefined : t("export.needModel"),
          onClick: () => setExportDialogOpen(true),
        },
        {
          kind: "item",
          label: t("menu.projectImage"),
          disabled: !hasMesh,
          hint: hasMesh ? undefined : t("export.needModel"),
          onClick: () => setProjectionOpen(true),
        },
        {
          kind: "item",
          label: t("menu.exportSplit"),
          disabled: !hasMesh,
          hint: hasMesh ? undefined : t("export.needModel"),
          onClick: () => {
            void (async () => {
              const dir = await openDialog({ directory: true });
              if (!dir) return;
              try {
                await exportSplitByColor(dir);
              } catch (e) {
                useAppStore.getState().setStatusMessage(`${t("toolbar.exportSplitFailed")}: ${e}`);
              }
            })();
          },
        },
        { kind: "sep" },
        // close() re-enters the onCloseRequested handler, so unexported work
        // still gets the exit confirmation; a clean state just closes.
        {
          kind: "item",
          label: t("menu.quit"),
          danger: true,
          onClick: () => {
            getCurrentWindow()
              .close()
              .catch((e) => {
                // Denied permission or a closed window must not fail silently —
                // GUI audit 2026-10-02 (B2): without this catch a missing
                // capability made the Quit item a dead button in release.
                log.error("MenuBar", "window.close failed", { error: String(e) });
                useAppStore.getState().setStatusMessage(t("menu.quitFailed"));
              });
          },
        },
      ],
    },
    {
      id: "view",
      label: t("menu.view"),
      items: [
        // GUI audit round 3 (B18): the View menu previously held no camera
        // command at all — every 3D app puts a view reset here.
        { kind: "item", label: t("menu.resetView"), onClick: act(() => useAppStore.getState().bumpViewReset()) },
        { kind: "sep" },
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
        // Language-aware manual: the zh UI links the Chinese manual, the en
        // UI the English one (both exist in the deployed docs tree).
        { kind: "item", label: t("menu.userGuide"), onClick: () => openExternal(MANUAL_URL[language]) },
        { kind: "item", label: t("menu.examples"), onClick: () => openExternal(EXAMPLES_URL) },
        { kind: "item", label: t("menu.wiki"), onClick: () => openExternal(SITE_URL) },
        { kind: "sep" },
        // Manual update check — always available, ignores the auto-check
        // preference and version skips (asking is consent to see the answer).
        // The updater's own busy-mutex ignores clicks while a download runs.
        { kind: "item", label: t("menu.checkUpdates"), onClick: () => useUpdateStore.getState().startManualCheck() },
        { kind: "sep" },
        { kind: "item", label: t("menu.discussions"), onClick: () => openExternal(DISCUSSIONS_URL) },
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
                    style={{
                      ...styles.item,
                      ...(item.danger ? styles.itemDanger : {}),
                      ...(item.disabled ? styles.itemDisabled : {}),
                    }}
                    role="menuitem"
                    disabled={item.disabled}
                    title={item.hint}
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
      {projectionOpen && <ProjectionDialog onClose={() => setProjectionOpen(false)} />}
      {aboutOpen && <AboutDialog onClose={() => setAboutOpen(false)} />}
    </div>
  );
}

/// Conventional About box: name/version/stack/licence/repo, same overlay
/// language as the other dialogs. Esc and overlay-click both close. Carries
/// the auto-check toggle so "不再提醒" from the update dialog is always
/// reversible from a discoverable place (update-module plan, config symmetry).
function AboutDialog({ onClose }: { onClose: () => void }) {
  const t = useT();
  const autoCheckEnabled = useUpdateStore((s) => s.autoCheckEnabled);
  const setAutoCheckEnabled = useUpdateStore((s) => s.setAutoCheckEnabled);
  const showExperimental = useAppStore((s) => s.showExperimental);
  const setShowExperimental = useAppStore((s) => s.setShowExperimental);

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
        <label style={styles.toggleRow}>
          <input
            type="checkbox"
            checked={autoCheckEnabled}
            onChange={(e) => setAutoCheckEnabled(e.target.checked)}
          />
          <span>{t("update.autoCheckToggle")}</span>
        </label>
        <label style={styles.toggleRow}>
          <input
            type="checkbox"
            checked={showExperimental}
            onChange={(e) => setShowExperimental(e.target.checked)}
          />
          <span>{t("about.showExperimental")}</span>
        </label>
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
  itemDisabled: {
    color: "var(--text-3, #777)",
    cursor: "not-allowed",
    opacity: 0.6,
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
  toggleRow: {
    display: "flex",
    alignItems: "center",
    gap: 6,
    marginTop: 10,
    fontSize: 12,
    color: "var(--text-2, #aaa)",
    cursor: "pointer",
    userSelect: "none",
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
