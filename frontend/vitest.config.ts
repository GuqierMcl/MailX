import { defineConfig, mergeConfig } from 'vitest/config'
import viteConfig from './vite.config.ts'

export default mergeConfig(viteConfig, defineConfig({
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    restoreMocks: true,
    include: ['src/**/*.test.{ts,tsx}'],
    // Whole-workbench tests take 2–4 s alone and run many at once; 5 s (the default)
    // made them fail on busy machines and CI runners without anything being wrong.
    testTimeout: 15_000,
    // Vitest blanks CSS by default, ?raw included; dnd-kit.test.tsx reads this one.
    css: { include: [/styles\/dnd-kit\.css/] },
  },
}))
