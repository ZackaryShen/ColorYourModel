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
  }

  info(tag: string, msg: string, data?: unknown) {
    if (!ENABLED_LEVELS.has("info")) return;
    console.log(fmt("info", tag, msg, data), `color: ${COLORS.info}; font-weight: bold`);
  }

  warn(tag: string, msg: string, data?: unknown) {
    if (!ENABLED_LEVELS.has("warn")) return;
    console.warn(fmt("warn", tag, msg, data), `color: ${COLORS.warn}`);
  }

  error(tag: string, msg: string, data?: unknown) {
    if (!ENABLED_LEVELS.has("error")) return;
    console.error(fmt("error", tag, msg, data), `color: ${COLORS.error}`);
  }
}

export const log = new Logger();
