import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { readFileSync } from "node:fs";

const host = process.env.TAURI_DEV_HOST;

// Single source of truth for the app version: tauri.conf.json. It is the same
// value the Rust updater compares against (app.package_info) and the one the
// NSIS installer stamps, so the About box can never disagree with the updater.
// (A package.json-only bump used to make the updater re-prompt forever —
// update-module REFUTE M8.)
const tauriConf = JSON.parse(readFileSync("./src-tauri/tauri.conf.json", "utf8")) as {
  version: string;
};

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  // Exposed to the About dialog (and anywhere else the app version shows).
  define: {
    __APP_VERSION__: JSON.stringify(tauriConf.version),
  },
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});
