import { test, expect } from '@playwright/test';
import { installStubs } from './stubs';
import { scanA11y, tabWalk, horizontalOverflow, currentFocus } from './helpers';

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
    dim.style.color = '#f1f3f4';
    dim.style.backgroundColor = '#fff';
    document.body.appendChild(dim); // color-contrast violation independent of the app theme
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

test('gate integrity: keyboard ceiling and identical labels cannot pass early', async ({ page }) => {
  await page.setContent('<style>button:focus { outline: 2px solid blue; }</style><button>Same</button><button>Same</button><button>Last</button>');
  await expect(tabWalk(page, 1)).rejects.toThrow('exceeded');
  const stops = await tabWalk(page);
  expect(stops.map(stop => stop.identity).sort()).toEqual(['Last', 'Same', 'Same']);
  const reverse = await tabWalk(page, 120, 'Shift+Tab');
  expect(reverse.map(stop => stop.identity).sort()).toEqual(['Last', 'Same', 'Same']);
});

test('gate integrity: clipped left content and obscured focus are detected', async ({ page }) => {
  await page.setContent('<button style="position:fixed;left:-30px">Clipped</button>');
  expect(await horizontalOverflow(page)).not.toEqual([]);
  await page.keyboard.press('Tab');
  expect((await currentFocus(page))?.inViewport).toBe(false);
  await page.setContent('<button>Covered</button><div style="position:fixed;inset:0;background:white"></div>');
  await page.keyboard.press('Tab');
  expect((await currentFocus(page))?.inViewport).toBe(false);
});
