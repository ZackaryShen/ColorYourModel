//! Bridge that lets the frontend report JavaScript runtime errors back to Rust
//! so they survive in headless / release builds where the browser DevTools
//! console is unavailable. The inline error handler in `index.html` calls
//! `report_js_error`; `report_app_ready` confirms React mounted successfully.
//!
//! Both print to stdout/stderr (captured when the app is launched from a
//! terminal) and append to a log file under the system temp dir.

use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

fn log_path() -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push("cym_js_diag.log");
    p
}

fn append(kind: &str, msg: &str) {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let line = format!("[{}] {}: {}\n", ts, kind, msg);

    // Always surface to the process stdio — visible when launched from a shell.
    eprintln!("CYM_JS_{}: {}", kind, msg);

    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
    {
        let _ = f.write_all(line.as_bytes());
    }
}

#[tauri::command]
pub fn report_js_error(msg: String) {
    append("ERROR", &msg);
}

#[tauri::command]
pub fn report_app_ready() {
    append("READY", "React tree mounted without throwing");
}
