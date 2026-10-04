import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useAppStore } from "../../store/appStore";
import { useUpdateStore } from "../../store/updateStore";
import { useT } from "../../i18n";
import { log } from "../../utils/logger";

const BYTES_PER_MB = 1048576;
const mb = (bytes: number) => (bytes / BYTES_PER_MB).toFixed(1);

/**
 * Update-module dialog: one component, one phase per render (idle renders
 * nothing). The download-progress listener lives HERE (not in the store
 * module) so jsdom tests never touch Tauri and events are only subscribed
 * while a download is actually running (REFUTE M9).
 *
 * Esc/overlay dismissal follows the other dialogs, EXCEPT: checking (the
 * result must land somewhere — dismissing would let it pop up unannounced
 * later), downloading (cancel is an explicit button), and installing (the
 * process is leaving; a half-dismissed terminal state is worse than a
 * locked dialog for its ~1s lifetime).
 */
export function UpdateDialog() {
  const t = useT();
  const phase = useUpdateStore((s) => s.phase);
  const info = useUpdateStore((s) => s.info);
  const errorReason = useUpdateStore((s) => s.errorReason);
  const errorFrom = useUpdateStore((s) => s.errorFrom);
  const progress = useUpdateStore((s) => s.progress);
  const paintDirty = useAppStore((s) => s.paintDirty);
  const hasMesh = useAppStore((s) => s.meshData !== null);

  const store = useUpdateStore;

  useEffect(() => {
    if (phase !== "downloading") return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    listen<{ received: number; total: number }>("update:download-progress", (e) => {
      store.getState().setProgress(e.payload.received, e.payload.total);
    })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch((e) => log.error("update", "progress listen failed", { error: String(e) }));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [phase, store]);

  // Esc = stay, on the phases where dismissing is safe at all.
  useEffect(() => {
    const closable = phase === "available" || phase === "upToDate" || phase === "error";
    if (!closable) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") store.getState().later();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [phase, store]);

  if (phase === "idle") return null;

  const dismissable = phase === "available" || phase === "upToDate" || phase === "error";

  // One line, reused: updating closes the app, unexported paint dies with it.
  const unsavedHint =
    (phase === "available" || phase === "ready") && paintDirty && hasMesh ? (
      <div style={styles.warnHint}>⚠️ {t("update.unsavedWorkHint")}</div>
    ) : null;

  let body: React.ReactNode = null;

  if (phase === "checking") {
    body = (
      <>
        <div style={styles.body}>{t("update.checking")}</div>
      </>
    );
  } else if (phase === "upToDate") {
    body = (
      <>
        <div style={styles.titleRow}>
          <span style={{ fontSize: 20 }} aria-hidden>
            ✅
          </span>
          <span style={styles.title}>{t("update.upToDateTitle")}</span>
        </div>
        <div style={styles.body}>{t("update.upToDateBody", info?.current ?? __APP_VERSION__)}</div>
      </>
    );
  } else if (phase === "available" && info) {
    body = (
      <>
        <div style={styles.titleRow}>
          <span style={{ fontSize: 20 }} aria-hidden>
            🎉
          </span>
          <span style={styles.title}>{t("update.availableTitle", `v${info.latest}`)}</span>
        </div>
        <div style={styles.body}>
          {t("update.currentVersion", `v${info.current}`)}
          {info.assetSize > 0 ? ` · ${t("update.installerSize", mb(info.assetSize))}` : ""}
        </div>
        {info.notes ? (
          <div style={styles.notes} role="note">
            {info.notes}
          </div>
        ) : null}
        {unsavedHint}
        <div style={styles.btnRow}>
          {info.canAutoInstall ? (
            <button className="cym-btn" style={styles.btnPrimary} onClick={() => store.getState().updateNow()}>
              {t("update.updateNow")}
            </button>
          ) : (
            <button
              className="cym-btn"
              style={styles.btnPrimary}
              onClick={() => info.releaseUrl && openUrl(info.releaseUrl).catch((e) => log.error("update", "openUrl failed", { error: String(e) }))}
            >
              {t("update.goToRelease")}
            </button>
          )}
          <button className="cym-btn" style={styles.btnSecondary} onClick={() => store.getState().later()}>
            {t("update.later")}
          </button>
        </div>
        <div style={styles.linkRow}>
          <button className="cym-btn" style={styles.btnLink} onClick={() => store.getState().skipVersion()}>
            {t("update.skipVersion")}
          </button>
          <button className="cym-btn" style={styles.btnLink} onClick={() => store.getState().neverRemind()}>
            {t("update.neverRemind")}
          </button>
        </div>
      </>
    );
  } else if (phase === "downloading") {
    const pct = progress.total > 0 ? Math.min(100, (progress.received / progress.total) * 100) : 0;
    body = (
      <>
        <div style={styles.titleRow}>
          <span style={{ fontSize: 20 }} aria-hidden>
            ⬇️
          </span>
          <span style={styles.title}>{t("update.downloading")}</span>
        </div>
        <div style={styles.track} role="progressbar" aria-valuenow={Math.round(pct)} aria-valuemin={0} aria-valuemax={100}>
          <div style={{ ...styles.fill, width: `${pct}%` }} />
        </div>
        <div style={styles.body}>
          {progress.total > 0
            ? t("update.progressMb", mb(progress.received), mb(progress.total))
            : t("update.progressBare", mb(progress.received))}
        </div>
        <div style={styles.btnRow}>
          <button className="cym-btn" style={styles.btnSecondary} onClick={() => store.getState().cancelDownload()}>
            {t("update.cancel")}
          </button>
        </div>
      </>
    );
  } else if (phase === "ready" && info) {
    body = (
      <>
        <div style={styles.titleRow}>
          <span style={{ fontSize: 20 }} aria-hidden>
            📦
          </span>
          <span style={styles.title}>{t("update.downloadComplete")}</span>
        </div>
        <div style={styles.body}>{t("update.readyBody", `v${info.latest}`)}</div>
        {unsavedHint}
        <div style={styles.btnRow}>
          <button className="cym-btn" style={styles.btnPrimary} onClick={() => store.getState().installNow()}>
            {t("update.installNow")}
          </button>
          <button className="cym-btn" style={styles.btnSecondary} onClick={() => store.getState().discardAndClose()}>
            {t("update.cancel")}
          </button>
        </div>
      </>
    );
  } else if (phase === "installing") {
    body = (
      <>
        <div style={styles.titleRow}>
          <span style={{ fontSize: 20 }} aria-hidden>
            ⚙️
          </span>
          <span style={styles.title}>{t("update.installingTitle")}</span>
        </div>
        <div style={styles.body}>{t("update.installing")}</div>
      </>
    );
  } else if (phase === "error") {
    body = (
      <>
        <div style={styles.titleRow}>
          <span style={{ fontSize: 20 }} aria-hidden>
            ❌
          </span>
          <span style={styles.title}>{t("update.failedTitle")}</span>
        </div>
        <div style={styles.body}>{errorReason ?? t("update.failedGeneric")}</div>
        {unsavedHint}
        <div style={styles.btnRow}>
          {/* Retry re-runs the step that actually failed: re-check for a check
              failure, re-download for a download failure. Install failures get
              no retry (the launcher already refused) — page + close only. */}
          {errorFrom === "download" ? (
            <button className="cym-btn" style={styles.btnPrimary} onClick={() => store.getState().updateNow()}>
              {t("update.retry")}
            </button>
          ) : errorFrom === "check" ? (
            <button className="cym-btn" style={styles.btnPrimary} onClick={() => store.getState().startManualCheck()}>
              {t("update.retry")}
            </button>
          ) : null}
          {info?.releaseUrl ? (
            <button
              className="cym-btn"
              style={styles.btnSecondary}
              onClick={() => info.releaseUrl && openUrl(info.releaseUrl).catch((e) => log.error("update", "openUrl failed", { error: String(e) }))}
            >
              {t("update.goToRelease")}
            </button>
          ) : null}
          <button className="cym-btn" style={styles.btnSecondary} onClick={() => store.getState().dismiss()}>
            {t("update.close")}
          </button>
        </div>
      </>
    );
  }

  return (
    <div
      style={styles.overlay}
      onClick={dismissable ? () => store.getState().later() : undefined}
    >
      <div
        style={styles.dialog}
        onClick={(e) => e.stopPropagation()}
        role={phase === "installing" ? "alertdialog" : "dialog"}
        aria-modal="true"
        aria-live={phase === "installing" ? "assertive" : undefined}
      >
        {body}
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
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
    width: 420,
    background: "var(--bg-panel, #2d2d2d)",
    border: "1px solid var(--border, #555)",
    borderRadius: 10,
    padding: 18,
    color: "var(--text-1, #eee)",
    fontFamily: "inherit",
    boxShadow: "0 8px 32px rgba(0,0,0,0.45)",
  },
  titleRow: {
    display: "flex",
    alignItems: "center",
    gap: 8,
    marginBottom: 6,
  },
  title: {
    fontSize: 15,
    fontWeight: 700,
  },
  body: {
    fontSize: 12.5,
    color: "var(--text-2, #aaa)",
    lineHeight: 1.6,
    marginTop: 4,
  },
  notes: {
    marginTop: 10,
    maxHeight: 160,
    overflowY: "auto",
    whiteSpace: "pre-wrap",
    wordBreak: "break-word",
    fontSize: 12,
    lineHeight: 1.55,
    color: "var(--text-2, #aaa)",
    background: "var(--bg-elevated, #252525)",
    border: "1px solid var(--border, #555)",
    borderRadius: 6,
    padding: "8px 10px",
  },
  warnHint: {
    marginTop: 10,
    fontSize: 12,
    lineHeight: 1.5,
    color: "var(--warning, #e0a94c)",
  },
  track: {
    marginTop: 12,
    height: 8,
    borderRadius: 4,
    background: "var(--bg-elevated, #252525)",
    border: "1px solid var(--border, #555)",
    overflow: "hidden",
  },
  fill: {
    height: "100%",
    background: "var(--accent, #3a5a7a)",
    transition: "width 120ms linear",
  },
  btnRow: {
    display: "flex",
    justifyContent: "flex-end",
    gap: 8,
    marginTop: 14,
  },
  linkRow: {
    display: "flex",
    justifyContent: "flex-end",
    gap: 14,
    marginTop: 10,
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
  btnSecondary: {
    padding: "6px 14px",
    border: "1px solid var(--border, #555)",
    background: "transparent",
    color: "var(--text-1, #eee)",
    borderRadius: 6,
    cursor: "pointer",
    fontSize: 13,
  },
  btnLink: {
    padding: 0,
    border: "none",
    background: "transparent",
    color: "var(--text-3, #888)",
    cursor: "pointer",
    fontSize: 12,
    textDecoration: "underline",
  },
};
