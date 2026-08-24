import { expect, test } from '@playwright/test';
import { installStubs } from './stubs';
import { expectNoHorizontalOverflow } from './helpers';

// WCAG 1.4.10 reflow proxy at the 320px minimum target width. The assertion
// measures every element's bounding rect, so body { overflow-x: hidden }
// cannot mask overflow.

const PAGES: Array<[string, string, (page: import('@playwright/test').Page) => Promise<void>]> = [
  ['home', '/', async () => {}],
  ['login', '/login', async () => {}],
  ['shelf', '/shelf', async (page) => {
    await page.getByText('星海拾遗').first().waitFor();
  }],
  ['reader with open world', '/reader/novel-1/1', async (page) => {
    await page.getByText('第一章 北塔来信').first().waitFor();
    await page.getByText(/的开放世界/).first().waitFor();
    const choice = page.getByText(/长选择起点/);
    const projection = page.getByText(/长行动投影起点/);
    await expect(choice).toHaveCSS('white-space', 'pre-wrap');
    await expect(choice).toHaveCSS('overflow-wrap', 'anywhere');
    await expect(projection).toHaveCSS('white-space', 'pre-wrap');
    await expect(projection).toHaveCSS('overflow-wrap', 'anywhere');
  }],
  ['characters', '/characters/novel-1', async (page) => {
    await page.getByRole('button', { name: /对话/ }).first().waitFor();
  }],
  ['settings', '/settings', async (page) => {
    await page.getByText('模型设置').first().waitFor();
  }],
  ['setup', '/', async (page) => {
    await page.getByText('欢迎使用 NovelWorld').first().waitFor();
  }],
];

test.describe('320px reflow — no horizontal overflow', () => {
  for (const [label, path, waitFor] of PAGES) {
    test(label, async ({ page }) => {
      await installStubs(page, { openWorld: label === 'reader with open world', setupNeeded: label === 'setup' });
      await page.setViewportSize({ width: 320, height: 720 });
      await page.goto(path);
      await waitFor(page);
      await page.waitForLoadState('networkidle');
      await expectNoHorizontalOverflow(page);
    });
  }
});
