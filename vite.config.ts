import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  optimizeDeps: {
    entries: ["index.html"],
  },
  server: {
    port: 1420,
    strictPort: true,
  },
});
