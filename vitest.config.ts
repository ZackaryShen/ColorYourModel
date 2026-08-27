import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Test-only Vite config (separate from vite.config.ts so it never affects the
// production bundle). The Tauri `invoke` bridge is mocked per-test file.
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.test.tsx", "src/**/*.test.ts"],
  },
});
