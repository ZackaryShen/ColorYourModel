/**
 * Structured logging utility for the ColorYourModel frontend.
 *
 * Usage:
 *   log.info("Model loaded", { vertices: 1000, faces: 500 });
 *   log.warn("GPU picker missed", { pixel, rect });
 *   log.error("STL load failed", err);
 *   log.debug("face picked", { faceId: 42 });
 */

type LogLevel = "debug" | "info" | "warn" | "error";

const COLORS: Record<LogLevel, string> = {
  debug: "#888",
  info: "#4a9eff",
  warn: "#ffa500",
  error: "#ff4444",
};

const ENABLED_LEVELS: Set<LogLevel> = new Set(["debug", "info", "warn", "error"]);

// ── In-app log ring buffer ───────────────────────────────────────
// Tauri release builds disable the webview devtools (Ctrl+Shift+I), so the
// console.* calls below are invisible to users. We ALSO keep a bounded ring
// buffer of the last N entries that a DebugLogViewer component can render
// directly in the app — iteration 58: this is the only way the user can read
// the seed-click diagnostics without devtools.
export interface LogEntry {
  ts: string;
  level: LogLevel;
  tag: string;
  msg: string;
  data?: unknown;
}

const RING_CAP = 300;
const ring: LogEntry[] = [];
const listeners = new Set<(entries: LogEntry[]) => void>();

function pushRing(level: LogLevel, tag: string, msg: string, data?: unknown) {
  const ts = new Date().toISOString().slice(11, 23); // HH:MM:SS.mmm
  ring.push({ ts, level, tag, msg, data });
  if (ring.length > RING_CAP) ring.splice(0, ring.length - RING_CAP);
  const snapshot = ring.slice();
  listeners.forEach((fn) => fn(snapshot));
}

/** Subscribe to the log ring. Returns an unsubscribe fn. */
export function subscribeLog(fn: (entries: LogEntry[]) => void): () => void {
  listeners.add(fn);
  fn(ring.slice());
  return () => {
    listeners.delete(fn);
  };
}

function fmt(level: LogLevel, tag: string, msg: string, data?: unknown): string {
  const ts = new Date().toISOString().slice(11, 23); // HH:MM:SS.mmm
  const prefix = `%c[${ts}] [${tag}] ${msg}`;
  if (data !== undefined) {
    return `${prefix} ${JSON.stringify(data, null, 0)}`;
  }
  return prefix;
}

class Logger {
  debug(tag: string, msg: string, data?: unknown) {
    if (!ENABLED_LEVELS.has("debug")) return;
    console.log(fmt("debug", tag, msg, data), `color: ${COLORS.debug}`);
    pushRing("debug", tag, msg, data);
  }

  info(tag: string, msg: string, data?: unknown) {
    if (!ENABLED_LEVELS.has("info")) return;
    console.log(fmt("info", tag, msg, data), `color: ${COLORS.info}; font-weight: bold`);
    pushRing("info", tag, msg, data);
  }

  warn(tag: string, msg: string, data?: unknown) {
    if (!ENABLED_LEVELS.has("warn")) return;
    console.warn(fmt("warn", tag, msg, data), `color: ${COLORS.warn}`);
    pushRing("warn", tag, msg, data);
  }

  error(tag: string, msg: string, data?: unknown) {
    if (!ENABLED_LEVELS.has("error")) return;
    console.error(fmt("error", tag, msg, data), `color: ${COLORS.error}`);
    pushRing("error", tag, msg, data);
  }
}

export const log = new Logger();
