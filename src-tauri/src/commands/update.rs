//! Update module: check GitHub Releases for a newer version, download the
//! Windows NSIS installer with progress events, and hand off to it for an
//! automatic (passive) install + relaunch.
//!
//! Design contract (adversarial plan `.zcode/update-module-plan.md`):
//! - `check_update` NEVER hard-fails: IPC-level trouble returns Err, but every
//!   HTTP/parse/compare outcome is a structured `UpdateStatus` so the frontend
//!   can always render something. Unparseable versions degrade to
//!   `CheckFailed`, never to a false "update available" prompt.
//! - `install_update` takes no path from the webview: it only ever launches
//!   the file recorded by a successful `download_update` (pending_install).
//! - Downloads are mutually exclusive (try_lock gate); cancel is polled per
//!   chunk and removes the partial file.
//! - All published version comparisons use the NORMALIZED (no `v` prefix)
//!   semver form; the frontend `skippedVersion` pref stores the same form.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use semver::Version;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

/// GitHub repo whose `/releases/latest` is polled. One constant because the
/// repository URL in Cargo.toml / MenuBar REPO_URL must stay in sync with it.
const RELEASES_LATEST_URL: &str =
    "https://api.github.com/repos/ZackaryShen/ColorYourModel/releases/latest";

/// GitHub's API rejects requests without a User-Agent header outright (403).
const USER_AGENT_PREFIX: &str = "ColorYourModel";

/// Overall budget for the `/releases/latest` round trip. The auto-check runs
/// silently at startup, so a hung connection must never outlive this window.
/// 15s ≈ 3x a healthy TLS+API round trip on a slow mobile link.
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);

/// connect(10s) covers TLS handshakes on lossy links; read_timeout(30s) is the
/// stall detector for both the check and the download stream.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Progress events are throttled to ~10/s; the download loop receives chunks
/// far more often and emitting every chunk floods the IPC for zero UX gain.
const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(100);

// ── Wire types ──────────────────────────────────────────────────────────────

/// Structured result of `check_update`. The `status` tag mirrors the frontend
/// discriminated union in `src/store/updateStore.ts`.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum UpdateStatus {
    UpToDate { current: String },
    Available {
        current: String,
        /// NORMALIZED (no `v` prefix) latest version — the exact form the
        /// frontend `skippedVersion` pref must be compared against.
        latest: String,
        notes: Option<String>,
        release_url: String,
        can_auto_install: bool,
        download_url: Option<String>,
        asset_size: u64,
    },
    CheckFailed { reason: String },
}

/// Result of `download_update`.
#[derive(Debug, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum DownloadOutcome {
    Done { path: String, size: u64 },
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
struct ProgressPayload {
    received: u64,
    total: u64,
}

/// Minimal view of the `/releases/latest` payload. Every field is optional or
/// defaulted ON PURPOSE: a missing `assets[].state` (etc.) must downgrade only
/// the auto-install candidate, never fail the whole parse (REFUTE M7).
#[derive(Debug, Deserialize)]
struct ReleasePayload {
    #[serde(default)]
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<ReleaseAssetPayload>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAssetPayload {
    #[serde(default)]
    name: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    browser_download_url: String,
    /// GitHub sets "uploaded" once the asset is complete; a missing field is
    /// treated as NOT installable (conservative) rather than a parse error.
    #[serde(default)]
    state: Option<String>,
}

// ── Pure helpers (unit-tested below) ────────────────────────────────────────

/// Strip the `v`/`V` release-tag prefix. Returns the normalized form only if
/// it starts with a digit — anything else is rejected so `semver::Version`
/// gets the final say downstream.
pub fn normalize_tag(tag: &str) -> Option<String> {
    let t = tag.trim().trim_start_matches(['v', 'V']);
    if t.starts_with(|c: char| c.is_ascii_digit()) {
        Some(t.to_string())
    } else {
        None
    }
}

/// How `latest_tag` relates to `current`. `Incomparable` (unparseable on
/// either side) is the honest "don't know" — callers must treat it as NO
/// update prompt, never as an update.
#[derive(Debug, PartialEq, Eq)]
pub enum VersionRelation {
    Older,
    Equal,
    Newer,
    Incomparable,
}

pub fn compare_versions(current: &str, latest_tag: &str) -> VersionRelation {
    let latest = normalize_tag(latest_tag).and_then(|t| Version::parse(&t).ok());
    let current = Version::parse(current).ok();
    match (current, latest) {
        (Some(c), Some(l)) => {
            if l > c {
                VersionRelation::Newer
            } else if l == c {
                VersionRelation::Equal
            } else {
                VersionRelation::Older
            }
        }
        _ => VersionRelation::Incomparable,
    }
}

/// Pick the auto-install asset: Windows NSIS installer whose upload
/// completed. Tauri's bundler names them `{productName}_{version}_{arch}-setup.exe`
/// (real v0.1.0 asset: `ColorYourModel_0.1.0_x64-setup.exe` — the arch is
/// UNDERSCORE-separated, which is why the suffix below is `_x64-setup.exe`;
/// the unit test encodes the real name). MSI is deliberately excluded
/// (per-machine MSI needs elevation, breaking the no-UAC auto flow); anything
/// else keeps the "open releases page" fallback alive. `is_windows` keeps the
/// decision pure and testable.
pub fn select_install_asset(
    is_windows: bool,
    assets: &[ReleaseAssetPayload],
) -> Option<&ReleaseAssetPayload> {
    if !is_windows {
        return None;
    }
    assets.iter().find(|a| {
        a.name.ends_with("_x64-setup.exe")
            && a.state.as_deref() == Some("uploaded")
            && a.browser_download_url.starts_with("https://")
    })
}

/// Reduce an asset filename to a safe single path component. Path traversal
/// (`../../evil.exe`) collapses to the bare name; anything that is not plain
/// `[A-Za-z0-9._-]` is rejected outright — GitHub asset names always pass.
pub fn sanitize_filename(name: &str) -> Option<String> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let ok = !base.is_empty()
        && base.len() <= 128
        && base != "."
        && base != ".."
        && base
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Some(base.to_string())
    } else {
        None
    }
}

// ── Managed state ───────────────────────────────────────────────────────────

pub struct UpdateState {
    client: reqwest::Client,
    /// Polled by the download loop between chunks; set by `cancel_update_download`.
    cancelled: AtomicBool,
    /// Excludes concurrent downloads (double-click on 更新, menu race, ...).
    /// try_lock — the second caller gets a clear error instead of queueing
    /// behind a 3.6 MB stream (REFUTE M2).
    download_gate: tokio::sync::Mutex<()>,
    /// The installer recorded by the last successful download. `install_update`
    /// refuses to run anything else, so the webview can never pick a path
    /// (REFUTE M3).
    pending_install: Mutex<Option<PathBuf>>,
}

impl UpdateState {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(format!("{USER_AGENT_PREFIX}/{}", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
            .build()
            .expect("update HTTP client construction cannot fail with static config");
        Self {
            client,
            cancelled: AtomicBool::new(false),
            download_gate: tokio::sync::Mutex::new(()),
            pending_install: Mutex::new(None),
        }
    }
}

// ── Commands ────────────────────────────────────────────────────────────────

/// Query GitHub for the latest published release and compare it with the
/// running version (from tauri.conf.json via package_info — the same source
/// the About box shows after the vite `__APP_VERSION__` unification).
#[tauri::command]
pub async fn check_update(
    app: AppHandle,
    state: State<'_, UpdateState>,
) -> Result<UpdateStatus, String> {
    let current = app.package_info().version.to_string();

    let send = state.client.get(RELEASES_LATEST_URL).send();
    let response = match tokio::time::timeout(CHECK_TIMEOUT, send).await {
        Err(_) => {
            return Ok(UpdateStatus::CheckFailed {
                reason: format!("request timed out after {}s", CHECK_TIMEOUT.as_secs()),
            })
        }
        Ok(Err(e)) => {
            return Ok(UpdateStatus::CheckFailed {
                reason: format!("network error: {e}"),
            })
        }
        Ok(Ok(r)) => r,
    };

    let status_code = response.status();
    if !status_code.is_success() {
        // 403/429: GitHub rate-limits unauthenticated API calls per IP; a
        // shared-IP user must get an honest reason, not a generic failure.
        let reason = if matches!(status_code.as_u16(), 403 | 429) {
            "GitHub API rate limited or blocked (HTTP 403/429) — try again later".to_string()
        } else {
            format!("GitHub API returned HTTP {}", status_code.as_u16())
        };
        return Ok(UpdateStatus::CheckFailed { reason });
    }

    let body = match response.text().await {
        Ok(t) => t,
        Err(e) => return Ok(UpdateStatus::CheckFailed { reason: format!("failed to read response: {e}") }),
    };
    let release: ReleasePayload = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(e) => {
            return Ok(UpdateStatus::CheckFailed {
                reason: format!("unexpected release payload: {e}"),
            })
        }
    };

    match compare_versions(&current, &release.tag_name) {
        VersionRelation::Newer => {
            let asset = select_install_asset(cfg!(windows), &release.assets);
            if asset.is_none() {
                // Keep a trace when a release ships without a usable installer
                // so an asset-rename regression is diagnosable from the log.
                log::warn!("no installable *_x64-setup.exe (state=uploaded) asset in latest release");
            }
            Ok(UpdateStatus::Available {
                current,
                latest: normalize_tag(&release.tag_name).unwrap_or_else(|| release.tag_name.clone()),
                notes: release.body,
                release_url: release.html_url,
                can_auto_install: asset.is_some(),
                download_url: asset.map(|a| a.browser_download_url.clone()),
                asset_size: asset.map(|a| a.size).unwrap_or(0),
            })
        }
        VersionRelation::Equal | VersionRelation::Older => Ok(UpdateStatus::UpToDate { current }),
        VersionRelation::Incomparable => Ok(UpdateStatus::CheckFailed {
            reason: format!(
                "cannot compare version {} with release tag {:?}",
                current, release.tag_name
            ),
        }),
    }
}

/// Stream the installer into `%TEMP%/cym-update/`, emitting
/// `update:download-progress` {received,total}. Cancellation is cooperative:
/// `cancel_update_download` flips a flag the chunk loop polls, and the partial
/// file is removed on every exit path (cancel, error, size mismatch).
#[tauri::command]
pub async fn download_update(
    app: AppHandle,
    state: State<'_, UpdateState>,
    url: String,
    filename: String,
    expected_size: u64,
) -> Result<DownloadOutcome, String> {
    // The URL comes from our own check_update response, but the webview is the
    // caller — pin the scheme so this command can never be aimed at file://
    // or an intranet http endpoint.
    if !url.starts_with("https://") {
        return Err("download url must be https".to_string());
    }
    let safe_name = sanitize_filename(&filename)
        .ok_or_else(|| format!("unsupported installer filename: {filename:?}"))?;

    let _gate = state
        .download_gate
        .try_lock()
        .map_err(|_| "a download is already in progress".to_string())?;
    state.cancelled.store(false, Ordering::SeqCst);

    let dir = std::env::temp_dir().join("cym-update");
    // Stale files from crashed/cancelled earlier sessions are wiped up front;
    // the gate guarantees no concurrent writer loses its file to this.
    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("cannot create update dir {}: {e}", dir.display()))?;
    let dest = dir.join(&safe_name);

    let mut response = state
        .client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("download failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("download failed: HTTP {}", response.status().as_u16()));
    }
    // Content-Length is authoritative; expected_size (from the release asset
    // listing) is only the fallback when the server streams without a length.
    let total = response.content_length().unwrap_or(expected_size);

    let mut file = tokio::io::BufWriter::new(
        tokio::fs::File::create(&dest)
            .await
            .map_err(|e| format!("cannot create {}: {e}", dest.display()))?,
    );
    let mut received: u64 = 0;
    let mut last_emit = Instant::now();
    let cleanup = |dest: &PathBuf| {
        // Best-effort partial-file removal; sync std call from async context is
        // fine for one small file and avoids nested async on error paths.
        let _ = std::fs::remove_file(dest);
    };

    loop {
        if state.cancelled.load(Ordering::SeqCst) {
            cleanup(&dest);
            return Ok(DownloadOutcome::Cancelled);
        }
        match response.chunk().await {
            Ok(Some(bytes)) => {
                use tokio::io::AsyncWriteExt;
                if let Err(e) = file.write_all(&bytes).await {
                    cleanup(&dest);
                    return Err(format!("write failed: {e}"));
                }
                received += bytes.len() as u64;
                if last_emit.elapsed() >= PROGRESS_EMIT_INTERVAL {
                    last_emit = Instant::now();
                    let _ = app.emit(
                        "update:download-progress",
                        ProgressPayload { received, total },
                    );
                }
            }
            Ok(None) => break,
            Err(e) => {
                cleanup(&dest);
                return Err(format!("download interrupted: {e}"));
            }
        }
    }
    {
        use tokio::io::AsyncWriteExt;
        if let Err(e) = file.flush().await {
            cleanup(&dest);
            return Err(format!("write failed: {e}"));
        }
    }
    drop(file);

    if total > 0 && received != total {
        cleanup(&dest);
        return Err(format!("size mismatch: got {received} bytes, expected {total}"));
    }

    *state.pending_install.lock().unwrap() = Some(dest.clone());
    // Final 100% event so the frontend bar lands full even under throttling.
    let _ = app.emit("update:download-progress", ProgressPayload { received, total });
    log::info!("update downloaded: {} ({} bytes)", dest.display(), received);
    Ok(DownloadOutcome::Done {
        path: dest.display().to_string(),
        size: received,
    })
}

/// Ask the in-flight download to stop. The `download_update` promise resolves
/// with `Cancelled` (partial file removed) shortly after.
#[tauri::command]
pub async fn cancel_update_download(state: State<'_, UpdateState>) -> Result<(), String> {
    state.cancelled.store(true, Ordering::SeqCst);
    Ok(())
}

/// Drop a downloaded installer without running it (frontend "cancel" on the
/// ready state). Best-effort file removal; the pending slot is always cleared
/// so `install_update` can never fire on a discarded file.
#[tauri::command]
pub async fn discard_update(state: State<'_, UpdateState>) -> Result<(), String> {
    // Bind BEFORE the if-let: a `lock().unwrap().take()` scrutinee temporary
    // would otherwise live across the `.await` below (std MutexGuard is !Send,
    // which poisons the command future — caught by cargo test).
    let discarded = state.pending_install.lock().unwrap().take();
    if let Some(path) = discarded {
        let _ = tokio::fs::remove_file(&path).await;
    }
    Ok(())
}

/// Launch the downloaded NSIS installer passively (`/P` progress bar, `/R`
/// relaunch-after-install — the exact flags Tauri's own updater uses) and
/// return immediately. The frontend destroys the window on return, so the
/// app is normally gone before the installer's app-running check executes;
/// if that race is ever lost, the NSIS template's passive branch shuts the
/// running app down via Restart Manager — identical to the official updater,
/// which is why no cmd.exe delay wrapper is needed (plan REFUTE B1/minor-11).
#[tauri::command]
pub async fn install_update(state: State<'_, UpdateState>) -> Result<(), String> {
    let path = state
        .pending_install
        .lock()
        .unwrap()
        .take()
        .ok_or("no downloaded update to install")?;
    if !path.is_file() {
        return Err(format!("installer file is missing: {}", path.display()));
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: harmless for the GUI-subsystem NSIS stub, keeps any
        // console spawned alongside it invisible.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new(&path)
            .args(["/P", "/R"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("failed to launch installer: {e}"))?;
        log::info!("installer launched: {}", path.display());
        Ok(())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        Err("automatic installation is only supported on Windows".to_string())
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_tag_strips_v_prefix_and_rejects_junk() {
        assert_eq!(normalize_tag("v0.2.0").as_deref(), Some("0.2.0"));
        assert_eq!(normalize_tag("V1.2.3").as_deref(), Some("1.2.3"));
        assert_eq!(normalize_tag("0.3.0").as_deref(), Some("0.3.0"));
        assert_eq!(normalize_tag("  v0.4.0  ").as_deref(), Some("0.4.0"));
        assert_eq!(normalize_tag("version-1"), None);
        assert_eq!(normalize_tag(""), None);
        assert_eq!(normalize_tag("release"), None);
    }

    #[test]
    fn compare_versions_covers_all_relations() {
        // Repo reality on day one: installed 0.1.1 > published v0.1.0.
        assert_eq!(compare_versions("0.1.1", "v0.1.0"), VersionRelation::Older);
        assert_eq!(compare_versions("0.1.0", "v0.1.1"), VersionRelation::Newer);
        assert_eq!(compare_versions("0.1.0", "v0.1.0"), VersionRelation::Equal);
        // Numeric, not lexicographic: 0.1.10 > 0.1.9.
        assert_eq!(compare_versions("0.1.9", "v0.1.10"), VersionRelation::Newer);
        assert_eq!(compare_versions("0.2.0", "v0.1.9"), VersionRelation::Older);
        // Unparseable → honest "don't know", never an update prompt.
        assert_eq!(compare_versions("0.1.1", "garbage"), VersionRelation::Incomparable);
        assert_eq!(compare_versions("", "v0.1.0"), VersionRelation::Incomparable);
    }

    fn asset(name: &str, size: u64, state: Option<&str>, url: &str) -> ReleaseAssetPayload {
        ReleaseAssetPayload {
            name: name.to_string(),
            size,
            browser_download_url: url.to_string(),
            state: state.map(|s| s.to_string()),
        }
    }

    #[test]
    fn select_install_asset_picks_uploaded_nsis_setup_only() {
        let assets = vec![
            asset(
                "ColorYourModel_0.2.0_x64_en-US.msi",
                5_529_600,
                Some("uploaded"),
                "https://github.com/a.msi",
            ),
            asset(
                "ColorYourModel_0.2.0_x64-setup.exe",
                3_590_475,
                Some("uploaded"),
                "https://github.com/setup.exe",
            ),
            asset(
                "ColorYourModel_0.2.0_x64-setup.exe",
                1,
                Some("starter"),
                "https://github.com/partial.exe",
            ),
        ];
        let picked = select_install_asset(true, &assets).expect("setup.exe must be picked");
        assert_eq!(picked.name, "ColorYourModel_0.2.0_x64-setup.exe");
        assert_eq!(picked.size, 3_590_475);

        // Missing state field (defaults to None) is NOT installable.
        let no_state = vec![asset(
            "ColorYourModel_0.2.0_x64-setup.exe",
            10,
            None,
            "https://github.com/setup.exe",
        )];
        assert!(select_install_asset(true, &no_state).is_none());

        // Non-Windows never auto-installs.
        assert!(select_install_asset(false, &assets).is_none());
    }

    #[test]
    fn parse_release_json_tolerates_missing_optional_fields() {
        // Real response shape (v0.1.0), with `body: null` and one asset whose
        // `state` field is absent — neither may fail the parse (REFUTE M7).
        let json = r#"{
            "tag_name": "v0.2.0",
            "name": "v0.2.0",
            "html_url": "https://github.com/ZackaryShen/ColorYourModel/releases/tag/v0.2.0",
            "body": null,
            "draft": false,
            "prerelease": false,
            "assets": [
                {
                    "name": "ColorYourModel_0.2.0_x64-setup.exe",
                    "size": 3590475,
                    "browser_download_url": "https://github.com/ZackaryShen/ColorYourModel/releases/download/v0.2.0/ColorYourModel_0.2.0_x64-setup.exe"
                }
            ]
        }"#;
        let release: ReleasePayload = serde_json::from_str(json).expect("must parse");
        assert_eq!(release.tag_name, "v0.2.0");
        assert!(release.body.is_none());
        assert_eq!(release.assets.len(), 1);
        assert_eq!(release.assets[0].state, None);
        let picked = select_install_asset(true, &release.assets);
        assert!(picked.is_none(), "asset without state must not auto-install");
    }

    #[test]
    fn sanitize_filename_blocks_traversal_and_junk() {
        assert_eq!(
            sanitize_filename("ColorYourModel_0.2.0_x64-setup.exe").as_deref(),
            Some("ColorYourModel_0.2.0_x64-setup.exe")
        );
        assert_eq!(
            sanitize_filename("../../evil.exe").as_deref(),
            Some("evil.exe"),
            "path components collapse to the bare name"
        );
        assert_eq!(sanitize_filename("a b.exe"), None, "spaces rejected");
        assert_eq!(sanitize_filename(""), None);
        assert_eq!(sanitize_filename(".."), None);
        assert_eq!(sanitize_filename("串.exe"), None, "non-ascii rejected");
    }

    /// Live proof that the whole check path works against the real endpoint:
    /// rustls roots trust api.github.com, the UA satisfies the API gate, and
    /// the payload deserializes into our struct with a comparable tag. Not part
    /// of the regular suite (network-dependent): run explicitly with
    /// `cargo test --lib live_check --release -- --ignored --nocapture`
    /// (repo convention for network-touching probes).
    #[test]
    #[ignore = "live network probe — run with -- --ignored"]
    fn live_check_github_api_parses_real_release() {
        let rt = tokio::runtime::Runtime::new().expect("test runtime");
        let release = rt.block_on(async {
            let client = reqwest::Client::builder()
                .user_agent(format!("{USER_AGENT_PREFIX}/{}", env!("CARGO_PKG_VERSION")))
                .connect_timeout(CONNECT_TIMEOUT)
                .read_timeout(READ_TIMEOUT)
                .build()
                .expect("client");
            let resp = client
                .get(RELEASES_LATEST_URL)
                .timeout(CHECK_TIMEOUT)
                .send()
                .await
                .expect("request must reach api.github.com");
            assert!(resp.status().is_success(), "HTTP {}", resp.status());
            let body = resp.text().await.expect("body");
            let release: ReleasePayload = serde_json::from_str(&body).expect("payload must parse");
            release
        });
        let relation = compare_versions("0.1.1", &release.tag_name);
        println!(
            "live release: tag={:?} assets={} relation_to_0.1.1={relation:?}",
            release.tag_name,
            release.assets.len(),
        );
        assert_ne!(
            relation,
            VersionRelation::Incomparable,
            "the real release tag must stay semver-comparable"
        );
    }
}
