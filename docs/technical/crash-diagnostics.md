# Crash Diagnostics

> Status: **placeholder** — content pending. Fill freely in English or 中文.

## Scope

How frontend failures get captured and surfaced instead of dying silently:

```mermaid
sequenceDiagram
    participant W as WebView (JS)
    participant B as Rust backend
    participant V as DebugLogViewer
    W->>B: report_js_error(detail)
    B->>B: persist to diagnostics log
    W->>V: open in-app diagnostics panel
```

Topics to cover: the `js_bridge` handshake (`report_js_error` / `report_app_ready`); in-app
`DebugLogViewer`; `RUST_LOG` backend logging; WebView2 gotchas on Windows (where console output
goes, how to attach devtools).

## Code pointers

- `src-tauri/src/commands/js_bridge.rs`
- `src/components/DebugLogViewer.tsx`
- `src/utils/logger.ts`

## TODO

- [ ] Diagnostic workflow: reproduce → capture → decode (with the 2026-08 crash case as example)
