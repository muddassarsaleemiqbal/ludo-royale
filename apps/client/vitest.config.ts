import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "../wasm/pkg/ludo_web.js": fileURLToPath(
        new URL("./src/test/mocks/ludo-web.ts", import.meta.url)
      )
    }
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
    coverage: {
      provider: "v8",
      reporter: ["text", "json-summary"],
      include: ["src/game/**/*.ts", "src/components/**/*.tsx"]
    }
  }
});
