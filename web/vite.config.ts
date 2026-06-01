import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Backend (moneywar-web actix) dev sırasında :8080'de koşar.
// /api ve /ws aynı origin'den proxy'lenir → frontend her zaman göreli yol kullanır.
const BACKEND = process.env.MONEYWAR_BACKEND ?? "http://localhost:8080";

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      "/api": { target: BACKEND, changeOrigin: true },
      "/ws": { target: BACKEND, ws: true, changeOrigin: true },
    },
  },
  build: {
    outDir: "dist",
    sourcemap: false,
  },
});
