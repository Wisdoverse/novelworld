import { test, expect } from '@playwright/test';
import { installStubs } from './stubs';
import { scanA11y } from './helpers';

// Anti-vacuous gate checks: the scan must FAIL CLOSED on injected
// violations, the catch-all stub must answer unstubbed routes, and a
// keyboard-focused control must show the :focus-visible indicator.

test('gate integrity: axe detects injected violations (fail-closed)', async ({ page }) => {
  await installStubs(page);
  await page.goto('/login');
  await page.waitForLoadState('networkidle');
  await page.evaluate(() => {
    const button = document.createElement('button');
    document.body.appendChild(button); // button-name violation: no text/aria-label
    const dim = document.createElement('p');
    dim.textContent = 'dim text';
    dim.style.color = '#334155';
    document.body.appendChild(dim); // color-contrast violation on the dark bg
  });
  const violations = await scanA11y(page);
  const ids = violations.map((v) => v.id);
  expect(ids, JSON.stringify(ids)).toContain('button-name');
  expect(ids, JSON.stringify(ids)).toContain('color-contrast');
});

test('gate integrity: unstubbed routes answer the catch-all stub', async ({ page }) => {
  await installStubs(page);
  await page.goto('/');
  const body = await page.evaluate(() =>
    fetch('/api/definitely-unstubbed').then((r) => r.json().catch(() => null)),
  );
  expect(body).toEqual({ code: 'stub_missing', message: expect.stringContaining('GET /definitely-unstubbed') });
});

test('gate integrity: a keyboard-focused control shows the focus indicator', async ({ page }) => {
  await installStubs(page);
  await page.goto('/settings');
  await page.waitForLoadState('networkidle');
  for (let i = 0; i < 60; i++) {
    const state = await page.evaluate(() => {
      const el = document.activeElement as HTMLElement | null;
      if (!el || el === document.body) return null;
      const cs = getComputedStyle(el);
      return {
        tag: el.tagName,
        cls: (el.className ?? '').slice(0, 70),
        outline: cs.outlineStyle + ' ' + cs.outlineWidth,
        shadow: cs.boxShadow,
      };
    });
    if (!state) {
      await page.keyboard.press('Tab');
      continue;
    }
    if (state.tag === 'SELECT') {
      expect(state.outline, JSON.stringify(state)).not.toBe('none 0px');
      return;
    }
    await page.keyboard.press('Tab');
  }
  throw new Error('never reached a SELECT in the tab order on /settings');
});
