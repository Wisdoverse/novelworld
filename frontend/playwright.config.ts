import { defineConfig } from '@playwright/test';

const previewPort = Number(process.env.PLAYWRIGHT_PORT ?? 4173);
const previewUrl = `http://127.0.0.1:${previewPort}`;

export default defineConfig({
  testDir: './e2e',
  timeout: 90_000,
  expect: { timeout: 10_000 },
  fullyParallel: false,
  workers: 1,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [['list']] : [['list']],
  use: {
    baseURL: previewUrl,
    headless: true,
    viewport: { width: 1280, height: 800 },
  },
  // The suite runs against the BUILT app (VITE_API_URL=/api) served by vite
  // preview — real index.html, real routing, real computed styles.
  webServer: {
    // --host 127.0.0.1 pins IPv4: on CI runners 'localhost' resolves to ::1
    // first and vite preview binds there, leaving the 127.0.0.1 health check
    // timing out.
    command: `node node_modules/vite/bin/vite.js preview --port ${previewPort} --strictPort --host 127.0.0.1`,
    url: previewUrl,
    reuseExistingServer: process.env.PLAYWRIGHT_REUSE_SERVER === 'true',
    timeout: 120_000,
  },
});
