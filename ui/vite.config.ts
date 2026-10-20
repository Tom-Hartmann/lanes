import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Tauri expects a fixed dev port (see app/tauri.conf.json).
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { target: "es2022", sourcemap: false },
});
