import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import { env } from "node:process";

const memoryApi = env.MEMORY_API_URL || "http://127.0.0.1:8080";
const terminal = env.PI_TERMINAL_URL || "http://127.0.0.1:7681";

export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      "/term": { target: terminal, ws: true },
      "/api": memoryApi,
      "/healthz": memoryApi,
      "/metrics": memoryApi,
      "/swagger-ui": memoryApi,
    },
  },
  build: {
    rollupOptions: {
      output: {
        entryFileNames: "assets/app.js",
        chunkFileNames: "assets/[name].js",
        assetFileNames: "assets/app.[ext]",
      },
    },
  },
  test: {
    environment: "jsdom",
    clearMocks: true,
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
