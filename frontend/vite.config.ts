import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import tailwindcss from "@tailwindcss/vite";

const configDirectory = dirname(fileURLToPath(import.meta.url));

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": resolve(configDirectory, "./src"),
    },
  },
});
