import { defineConfig } from "vite";

export default defineConfig({
  server: {
    port: 7807,
    proxy: {
      "/api": { target: "http://127.0.0.1:7806", changeOrigin: true },
      "/vault": { target: "http://127.0.0.1:7806", changeOrigin: true },
      "/ws": { target: "ws://127.0.0.1:7806", ws: true },
    },
  },
  build: {
    outDir: "dist",
    chunkSizeWarningLimit: 1500,
  },
});
