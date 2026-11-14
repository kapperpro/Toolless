import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Vite-Konfiguration für Tauri: fester Port, keine Überwachung des Rust-Codes.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/target/**"] },
  },
  build: {
    target: "es2021",
    outDir: "dist",
  },
});
