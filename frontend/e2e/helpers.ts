import { expect, type Page, type Locator } from '@playwright/test';

export interface AxeViolation {
  id: string;
  help: string;
  impact: string;
  targets: string[];
  html: string;
  summary: string;
}

declare global {
  interface Window {
    axe: {
      run: (context: unknown, options: unknown) => Promise<{
        violations: Array<{
          id: string;
          help: string;
          impact: string;
          nodes: Array<{ target: string[]; html: string; failureSummary: string }>;
        }>;
      }>;
    };
  }
}

/** Full WCAG 2.2 AA rule set, including color-contrast, on the real DOM. */
export async function scanA11y(page: Page): Promise<AxeViolation[]> {
  await page.addScriptTag({ path: 'node_modules/axe-core/axe.min.js' });
  // Deterministic typography gate: wait for font loading to settle (fallback
  // set in CI and locally — Google Fonts is aborted by the stub layer).
  await page.evaluate(() => document.fonts.ready);
  return page.evaluate(async () => {
    const result = await window.axe.run(document, {
      resultTypes: ['violations'],
      runOnly: { type: 'tag', values: ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'] },
    });
    return result.violations.map((v) => ({
      id: v.id,
      help: v.help,
      impact: v.impact,
      targets: v.nodes.map((n) => n.target.join(' ')),
      html: v.nodes.map((n) => n.html.slice(0, 220)).join(' || '),
      summary: v.nodes.map((n) => n.failureSummary ?? '').join(' || '),
    }));
  });
}

/** Wait until no element carries an in-progress framer-motion opacity
 * (inline opacity < 1), so scans never run mid-animation. */
export async function settleAnimations(page: Page): Promise<void> {
  await page.waitForFunction(() => {
    for (const el of Array.from(document.querySelectorAll('*'))) {
      // Radix uses permanently transparent focus sentinels to contain Tab
      // navigation. They are infrastructure, not an in-progress animation.
      if (el.hasAttribute('data-radix-focus-guard')) continue;
      const value = (el as HTMLElement).style.opacity;
      if (value && Number(value) < 1) return false;
    }
    return true;
  }, undefined, { timeout: 10_000 });
}

export async function expectNoA11yViolations(page: Page): Promise<void> {
  const violations = await scanA11y(page);
  const message = violations
    .map((v) => v.id + ' (' + v.impact + '): ' + v.help + '\n  ' + v.targets.join('\n  ')
      + '\n  html: ' + v.html + '\n  ' + v.summary.split('\n').join('\n  '))
    .join('\n');
  expect(violations, message).toEqual([]);
}

export interface FocusState {
  tag: string;
  identity: string;
  cls: string;
  text: string;
  role: string | null;
  visibleIndicator: boolean;
  inViewport: boolean;
}

export async function currentFocus(page: Page): Promise<FocusState | null> {
  return page.evaluate(() => {
    const el = document.activeElement;
    if (!el || el === document.body) return null;
    const cs = getComputedStyle(el);
    const rect = el.getBoundingClientRect();
    const hasOutline = cs.outlineStyle !== 'none' && cs.outlineWidth !== '0px';
    const hasShadow = cs.boxShadow !== 'none';
    const hit = document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2);
    return {
      tag: el.tagName.toLowerCase(),
      identity: el.getAttribute('id')
        ?? el.getAttribute('aria-label')
        ?? el.getAttribute('name')
        ?? el.getAttribute('placeholder')
        ?? ((el as HTMLElement).textContent ?? '').trim().slice(0, 40),
      cls: (el as HTMLElement).className?.slice(0, 80) ?? '',
      text: ((el as HTMLElement).textContent ?? '').trim().slice(0, 40),
      role: el.getAttribute('role'),
      visibleIndicator: hasOutline || hasShadow,
      inViewport: rect.width > 0 && rect.height > 0
        && cs.visibility === 'visible' && Number(cs.opacity) > 0
        && rect.top >= 0 && rect.bottom <= window.innerHeight
        && rect.left >= 0 && rect.right <= window.innerWidth
        && (hit === el || el.contains(hit)),
    };
  });
}

/**
 * Walk the real tab order with the native keyboard, collecting every focus
 * stop. Throws if the focus sequence cycles (trap) or runs too long.
 */
export async function tabWalk(page: Page, maxStops = 120, direction: 'Tab' | 'Shift+Tab' = 'Tab'): Promise<FocusState[]> {
  const stops: FocusState[] = [];
  const seen = await page.evaluateHandle(() => new Set<Element>());
  try {
    for (let i = 0; i < maxStops; i++) {
      await page.keyboard.press(direction);
      const state = await currentFocus(page);
      if (!state) continue; // Chromium's page-chrome stop between page cycles.
      const repeat = await seen.evaluate(elements => {
        const active = document.activeElement!;
        if (elements.has(active)) return active === elements.values().next().value ? 'first' : 'trap';
        elements.add(active);
        return null;
      });
      if (repeat === 'first') return stops;
      if (repeat === 'trap') throw new Error('Tab walk repeated an element before completing its cycle');
      expect(state.visibleIndicator, `focus indicator: ${state.identity}`).toBe(true);
      await expect.poll(async () => (await currentFocus(page))?.inViewport,
        { message: `focus outside viewport or obscured: ${state.identity}` }).toBe(true);
      stops.push(state);
    }
    throw new Error(`Tab walk exceeded ${maxStops} stops without a complete cycle`);
  } finally {
    await seen.dispose();
  }
}

/** Proves a named control is reachable, not merely that some cycle exists. */
export async function tabTo(page: Page, target: Locator, direction: 'Tab' | 'Shift+Tab' = 'Tab') {
  await expect(target).toBeVisible();
  await expect(target).toBeEnabled();
  for (let i = 0; i < 120; i++) {
    await page.keyboard.press(direction);
    const state = await currentFocus(page);
    if (!state) continue;
    expect(state.visibleIndicator, `focus indicator: ${state.identity}`).toBe(true);
    await expect.poll(async () => (await currentFocus(page))?.inViewport,
      { message: `focus outside viewport or obscured: ${state.identity}` }).toBe(true);
    if (await target.evaluateAll(els => els.some(el => el === document.activeElement))) return;
  }
  throw new Error('Target was not reached within 120 keyboard stops');
}

export interface OverflowItem {
  tag: string;
  cls: string;
  left: number;
  right: number;
}

/** Every element's rect must stay within the viewport width (reflow proxy,
 * immune to body { overflow-x: hidden } masking). */
export async function horizontalOverflow(page: Page): Promise<OverflowItem[]> {
  return page.evaluate(() => {
    const width = document.documentElement.clientWidth;
    const bad: OverflowItem[] = [];
    for (const el of Array.from(document.querySelectorAll('body *'))) {
      const rect = el.getBoundingClientRect();
      if (rect.width === 0 && rect.height === 0) continue;
      // Skip purely decorative background blobs: textless, non-interactive,
      // unnamed, alt-less, absolutely/fixed-positioned elements bleeding off
      // the edge are clipped by body overflow-x:hidden and never cause
      // scrolling. In-flow content (including wide images) is never skipped.
      const text = (el.textContent ?? '').trim();
      const interactive = el.matches('input, button, select, textarea, a, [tabindex]')
        || el.querySelector('input, button, select, textarea, a, [tabindex]');
      const named = el.getAttribute('aria-label') || el.getAttribute('role') || el.getAttribute('alt');
      const position = getComputedStyle(el).position;
      const decorative = !text && !interactive && !named
        && (position === 'absolute' || position === 'fixed');
      if (decorative) continue;
      // CSS-clipped sr-only labels are deliberately outside visual layout.
      if (getComputedStyle(el).clip === 'rect(0px, 0px, 0px, 0px)') continue;
      if (rect.left < -1 || rect.right > width + 1) {
        bad.push({
          tag: el.tagName.toLowerCase(),
          cls: (el as HTMLElement).className?.slice(0, 60) ?? '',
          left: Math.round(rect.left),
          right: Math.round(rect.right),
        });
        if (bad.length >= 20) break;
      }
    }
    return bad;
  });
}

export async function expectNoHorizontalOverflow(page: Page): Promise<void> {
  const bad = await horizontalOverflow(page);
  const message = bad.map((b) => b.tag + '.' + b.cls + ' left=' + b.left + ' right=' + b.right).join('\n');
  expect(bad, message).toEqual([]);
}
