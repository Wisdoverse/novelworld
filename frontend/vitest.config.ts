import { defineConfig } from 'vitest/config';
import { fileURLToPath } from 'node:url';

export default defineConfig({
  test: {
    environment: 'jsdom',
    globals: true,
    // Node 26 reserves process-global Web Storage even without a backing file.
    // Disable it in workers so Vitest can install jsdom's per-test storage.
    execArgv: ['--no-experimental-webstorage'],
    // The Playwright browser gate lives in e2e/ and is owned by its own runner.
    exclude: ['e2e/**', 'node_modules/**', 'dist/**'],
  },
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
});
