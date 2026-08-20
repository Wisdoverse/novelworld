import axe from 'axe-core';
import { expect } from 'vitest';

// jsdom cannot compute real styles: getComputedStyle returns literal
// var(--token) values and transparent backgrounds, so axe's
// color-contrast rule resolves to 'incomplete' here. Real contrast is
// enforced by the browser-based gate (recorded limitation on issue #165).
const JSDOM_DISABLED_RULES = { 'color-contrast': { enabled: false } };

export async function expectNoA11yViolations(container: HTMLElement): Promise<void> {
  const results = await axe.run(container, { rules: JSDOM_DISABLED_RULES });
  const violations = results.violations;
  expect(violations, violations
    .map(v => v.id + ': ' + v.help + ' (' + v.nodes.length + ' nodes) ' + v.helpUrl)
    .join('\n')).toEqual([]);
}