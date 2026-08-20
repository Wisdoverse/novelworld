import { expect, type Page } from '@playwright/test';

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
    return {
      tag: el.tagName.toLowerCase(),
      cls: (el as HTMLElement).className?.slice(0, 80) ?? '',
      text: ((el as HTMLElement).textContent ?? '').trim().slice(0, 40),
      role: el.getAttribute('role'),
      visibleIndicator: hasOutline || hasShadow,
      inViewport: rect.top >= 0 && rect.bottom <= window.innerHeight
        && rect.left >= 0 && rect.right <= window.innerWidth,
    };
  });
}

/**
 * Walk the real tab order with the native keyboard, collecting every focus
 * stop. Throws if the focus sequence cycles (trap) or runs too long.
 */
export async function tabWalk(page: Page, maxStops = 120): Promise<FocusState[]> {
  const stops: FocusState[] = [];
  const seen = new Set<string>();
  await page.keyboard.press('Tab');
  for (let i = 0; i < maxStops; i++) {
    const state = await currentFocus(page);
    if (!state) break;
    stops.push(state);
    const key = state.tag + '|' + state.text;
    if (seen.has(key) && stops.filter((s) => s.tag + '|' + s.text === key).length >= 3) {
      throw new Error('focus trap detected on: ' + JSON.stringify(state));
    }
    seen.add(key);
    await page.keyboard.press('Tab');
  }
  return stops;
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
      if (rect.right > width + 1 || rect.left < -1) {
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
