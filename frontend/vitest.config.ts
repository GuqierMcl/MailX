import { defineConfig, mergeConfig } from 'vitest/config'
import viteConfig from './vite.config.ts'

export default mergeConfig(viteConfig, defineConfig({
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    restoreMocks: true,
    include: ['src/**/*.test.{ts,tsx}'],
    // Vitest blanks CSS by default, ?raw included; dnd-kit.test.tsx reads this one.
    css: { include: [/styles\/dnd-kit\.css/] },
  },
}))
