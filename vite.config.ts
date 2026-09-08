import { resolve } from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // Never watch the Rust build output -- cargo locks/rewrites files in
      // here constantly, and Vite's fs watcher previously crashed the whole
      // dev server with an EBUSY error the instant `cargo build` touched the
      // binary while it was running.
      ignored: ["**/src-tauri/target/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2021",
    outDir: "dist",
    rollupOptions: {
      input: {
        main: resolve(import.meta.dirname, "index.html"),
        quickCapture: resolve(import.meta.dirname, "quick-capture.html"),
      },
    },
  },
});
