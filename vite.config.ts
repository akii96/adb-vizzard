import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

const projectRoot = import.meta.dirname;

// Tauri drives the dev server on a fixed port and watches src-tauri itself.
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { "@": path.resolve(projectRoot, "./src") },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "chrome105",
    sourcemap: false,
    chunkSizeWarningLimit: 1200,
    rollupOptions: {
      output: {
        // Keep the boot path tiny: charts and the export writer are dynamically
        // imported, so they must land in their own chunks rather than the entry.
        manualChunks(id) {
          if (id.includes("echarts") || id.includes("zrender")) return "charts";
          return undefined;
        },
      },
    },
  },
});
