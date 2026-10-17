import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import tailwindcss from "@tailwindcss/vite";
import { readFileSync } from "node:fs";

const configDirectory = dirname(fileURLToPath(import.meta.url));
// The root package.json is the workspace version source.
const appVersion = JSON.parse(readFileSync(resolve(configDirectory, "../package.json"), "utf8")).version;

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  define: { __APP_VERSION__: JSON.stringify(appVersion) },
  resolve: {
    alias: {
      "@": resolve(configDirectory, "./src"),
    },
  },
});
