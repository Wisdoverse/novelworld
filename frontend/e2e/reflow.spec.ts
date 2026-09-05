import { expect, test } from '@playwright/test';
import { installStubs } from './stubs';
import { expectNoHorizontalOverflow, settleAnimations, tabTo } from './helpers';

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
    await page.getByRole('heading', { name: '平台模型设置' }).waitFor();
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

for (const viewport of [{ width: 320, height: 720 }, { width: 568, height: 320 }, { width: 320, height: 256 }]) {
  test(`opened overlays reflow at ${viewport.width}x${viewport.height}`, async ({ page }, testInfo) => {
    await installStubs(page);
    await page.setViewportSize(viewport);
    await page.goto('/reader/novel-1/1');
    await page.getByRole('button', { name: '角色', exact: true }).click();
    const drawer = page.getByRole('dialog', { name: '故事角色' });
    await expect(drawer).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: testInfo.outputPath('drawer.png') });
    await drawer.getByRole('button', { name: /林晚/ }).click();
    const chat = page.getByRole('region', { name: '与 林晚 对话' });
    await settleAnimations(page);
    const input = chat.getByRole('textbox');
    await tabTo(page, input);
    await input.fill('一条用于检查窄屏输入的消息');
    await tabTo(page, chat.getByRole('button', { name: '发送消息' }));
    await expectNoHorizontalOverflow(page);
    const rect = await chat.boundingBox();
    expect(rect!.y).toBeGreaterThanOrEqual(0);
    expect(rect!.y + rect!.height).toBeLessThanOrEqual(viewport.height);
    await page.screenshot({ path: testInfo.outputPath('chat.png') });
    await page.keyboard.press('Escape');
    await expect(chat).toBeHidden();

    await page.goto('/shelf');
    for (const name of ['导入小说', '打开共享书库']) {
      await page.getByRole('button', { name }).click();
      const dialog = page.getByRole('dialog');
      await expect(dialog).toBeVisible();
      await expectNoHorizontalOverflow(page);
      await tabTo(page, dialog.getByRole('button', { name: name === '导入小说' ? '取消' : '完成', exact: true }));
      await page.keyboard.press('Escape');
      await expect(dialog).toBeHidden();
    }
  });
}

for (const initialPreference of ['reduce', 'no-preference'] as const) {
test(`reduced motion (${initialPreference} at load) skips runtime animation and both scroll paths`, async ({ page }) => {
  await page.emulateMedia({ reducedMotion: initialPreference });
  await installStubs(page, { openWorld: true });
  await page.addInitScript(() => {
    const calls: string[] = [];
    Object.defineProperty(window, 'scrollCalls', { value: calls });
    for (const method of ['scrollIntoView', 'scrollTo'] as const) {
      const original = Element.prototype[method];
      Object.defineProperty(Element.prototype, method, { value: function (options: ScrollIntoViewOptions) {
        calls.push(`${method}:${options?.behavior}`);
        return Reflect.apply(original, this, [options]);
      } });
    }
  });
  await page.goto('/reader/novel-1/1');
  await expect(page.getByRole('heading', { name: /的开放世界/ })).toBeVisible();
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await expect(page.locator('html')).toHaveCSS('scroll-behavior', 'auto');
  await page.getByRole('button', { name: '回看行动日志' }).click();
  await page.getByRole('button', { name: '角色', exact: true }).click();
  await page.getByRole('dialog').getByRole('button', { name: /林晚/ }).click();
  const chat = page.getByRole('region', { name: '与 林晚 对话' });
  await expect(chat).toHaveCSS('transform', 'none');
  await expect(chat).toHaveCSS('opacity', '1');
  await chat.getByRole('textbox').fill('你好');
  await chat.getByRole('textbox').press('Enter');
  await expect(page.getByRole('log', { name: '已保存的对话' })).toContainText('星光洒在海面上');
  const calls = await page.evaluate(() => (window as unknown as { scrollCalls: string[] }).scrollCalls);
  expect(calls).toContain('scrollIntoView:instant');
  expect(calls).toContain('scrollTo:instant');
  expect(calls.some(call => call.endsWith(':smooth'))).toBe(false);
});
}
