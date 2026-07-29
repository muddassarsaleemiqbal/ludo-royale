import { defineConfig } from "vite";
import react, { reactCompilerPreset } from '@vitejs/plugin-react';
import tailwindcss from "@tailwindcss/vite";
import babel from '@rolldown/plugin-babel';

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [
    react(),
    babel({
      presets: [reactCompilerPreset()]
    }),
    tailwindcss()
  ],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 5174 } : undefined,
    watch: {
      ignored: ["**/src-tauri/**"]
    }
  },
  build: {
    target: "es2022",
    sourcemap: true
  }
});
