import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// 7000 is the web app's named port in localenv.yaml (instance 0). `just infra` points the gateway's `/` here, so
// http://localhost:7080 serves this dev server and `/api` behind one origin, and emailed links land on it.
const port = Number(process.env.WEB_DEV_PORT) || 7000;

export default defineConfig({
  plugins: [react(), tailwindcss()],

  // backend-client ships TypeScript source: kept out of pre-bundling so edits to it show up at once, with its
  // dependency listed so it is bundled up front.
  optimizeDeps: {
    exclude: ["@simpled/backend-client"],
    include: ["zod"],
  },

  server: {
    port,
    strictPort: true,
    // Pages loaded through the gateway still get hot reload straight from here.
    hmr: { clientPort: port },
    // The workspace root: backend-client is a symlink outside this package.
    fs: { allow: [".."] },
    // For http://localhost:7000 opened directly: the gateway routes /api to main-svc.
    proxy: {
      "/api": { target: process.env.WEB_DEV_API_TARGET ?? "http://localhost:7080" },
    },
  },

  build: {
    sourcemap: true,
  },

  // Ops and controllers are plain TypeScript: no DOM.
  test: {
    include: ["src/**/*.test.ts"],
    environment: "node",
  },
});
