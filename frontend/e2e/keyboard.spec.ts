import { test, expect, type Locator } from '@playwright/test';
import { installStubs } from './stubs';
import { tabWalk } from './helpers';

// Keyboard operability on the critical journey: real Tab focus movement,
// visible focus indicators, Enter/Space activation, and focus containment.

async function expectFocusWithin(dialog: Locator) {
  await expect
    .poll(() => dialog.evaluate((element) => element.contains(element.ownerDocument.activeElement)))
    .toBe(true);
}

test.describe('critical journey — keyboard operability', () => {
  test('login: tab walk, visible focus, Enter submits and lands on the shelf', async ({ page }) => {
    await installStubs(page, { authenticated: false });
    await page.goto('/login');
    const email = page.getByRole('textbox').first();
    await email.fill('reader@example.com');
    await page.getByRole('textbox').nth(1).fill('secret-pass');

    const stops = await tabWalk(page);
    expect(stops.length).toBeGreaterThan(3);
    for (const stop of stops) {
      expect(stop.visibleIndicator, 'no visible focus indicator on ' + stop.tag + ' ' + stop.cls).toBe(true);
    }

    await email.focus();
    await page.keyboard.press('Enter');
    await expect(page).toHaveURL(/\/shelf/);
    await expect(page.getByText('星海拾遗').first()).toBeVisible();
  });

  test('shelf: novel card opens the reader', async ({ page }) => {
    await installStubs(page);
    await page.goto('/shelf');
    await expect(page.getByText('星海拾遗').first()).toBeVisible();
    const card = page.getByRole('button', { name: /^星海拾遗/ }).first();
    await card.focus();
    await page.keyboard.press('Enter');
    await expect(page).toHaveURL(/\/reader\/novel-1\/1/);
  });

  test('shelf: shared library traps focus, closes with Escape, and restores focus', async ({ page }) => {
    await installStubs(page);
    await page.goto('/shelf');
    const trigger = page.getByRole('button', { name: '打开共享书库' });
    await trigger.focus();
    await page.keyboard.press('Enter');

    const dialog = page.getByRole('dialog', { name: '共享书库' });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByRole('button', { name: '加入书架' }).first()).toBeVisible();
    await expectFocusWithin(dialog);
    for (let index = 0; index < 8; index += 1) {
      await page.keyboard.press('Tab');
      await expectFocusWithin(dialog);
    }

    await page.keyboard.press('Escape');
    await expect(dialog).toBeHidden();
    await expect(trigger).toBeFocused();

    const remove = page.getByRole('button', { name: '将 星海拾遗 移出书架' });
    await remove.focus();
    await expect(remove).toBeVisible();
    await expect(remove).toBeFocused();
  });

  test('shelf: import dialog traps focus, closes with Escape, and restores focus', async ({ page }) => {
    await installStubs(page);
    await page.goto('/shelf');
    const trigger = page.getByRole('button', { name: '导入小说' });
    await trigger.focus();
    await page.keyboard.press('Enter');

    const dialog = page.getByRole('dialog', { name: '导入小说' });
    await expect(dialog).toBeVisible();
    await expectFocusWithin(dialog);
    for (let index = 0; index < 8; index += 1) {
      await page.keyboard.press('Shift+Tab');
      await expectFocusWithin(dialog);
    }

    await page.keyboard.press('Escape');
    await expect(dialog).toBeHidden();
    await expect(trigger).toBeFocused();
  });

  test('reader: branch choice activates with Space and renders the consequence', async ({ page }) => {
    await installStubs(page);
    await page.goto('/reader/novel-1/1');
    const choice = page.getByRole('button', { name: /收下信/ }).first();
    await choice.focus();
    await page.keyboard.press('Space');
    await expect(page.getByText('旅人收下信，约定黎明出海。').first()).toBeVisible();
    await expect(choice).toHaveAttribute('aria-pressed', 'true');
  });

  test('reader: chat input sends with Enter (SSE stub commits)', async ({ page }) => {
    await installStubs(page);
    await page.goto('/reader/novel-1/1');
    await page.getByRole('button', { name: /角色/ }).first().click();
    await page.getByRole('button', { name: /林晚/ }).first().click();
    const input = page.getByRole('textbox', { name: /对 林晚 说/ });
    await input.fill('你好');
    await input.press('Enter');
    await expect(page.getByText('星光洒在海面上。').first()).toBeVisible();
  });

  test('world action form submits from the keyboard', async ({ page }) => {
    await installStubs(page, { openWorld: true });
    await page.goto('/reader/novel-1/1');
    await expect(page.getByText(/的开放世界/).first()).toBeVisible();
    const submit = page.getByRole('button', { name: /行动|提交|执行/ }).first();
    await submit.focus();
    await page.keyboard.press('Space');
    // The turn stub commits: the rendered narrative of the transition appears.
    await expect(page.getByText(/旅人沿山路下行/).first()).toBeVisible();
  });

  test('settings: tab walk stays in the page and every stop has a focus indicator', async ({ page }) => {
    await installStubs(page);
    await page.goto('/settings');
    await expect(page.getByRole('heading', { name: '平台模型设置' })).toBeVisible();
    const stops = await tabWalk(page);
    expect(stops.length).toBeGreaterThan(3);
    for (const stop of stops) {
      expect(stop.visibleIndicator, 'no visible focus indicator on ' + stop.tag + ' ' + stop.cls).toBe(true);
    }
  });

  test('player entry form submits from the keyboard', async ({ page }) => {
    await installStubs(page, { entryRequired: true });
    await page.goto('/reader/novel-1/1');
    const name = page.getByRole('textbox', { name: /名字|姓名|角色名/i }).first();
    await name.fill('测试旅人');
    await page.getByRole('textbox', { name: /背景/i }).fill('一个测试角色');
    await page.getByRole('textbox', { name: /能力/i }).fill('阅读');
    const submit = page.getByRole('button', { name: /创建|进入/ }).first();
    await submit.focus();
    await page.keyboard.press('Enter');
    // The form disappears and the world-enter gate appears once the entry is saved.
    await expect(page.getByRole('button', { name: /进入开放世界/ })).toBeVisible();
  });

  test('setup: tab walk and keyboard completion of the first-run form', async ({ page }) => {
    await installStubs(page, { setupNeeded: true });
    await page.goto('/');
    await expect(page.getByText('欢迎使用 NovelWorld').first()).toBeVisible();
    const stops = await tabWalk(page);
    expect(stops.length).toBeGreaterThan(3);
    for (const stop of stops) {
      expect(stop.visibleIndicator, 'no visible focus indicator on ' + stop.tag + ' ' + stop.cls).toBe(true);
    }
    await page.getByRole('textbox', { name: /邮箱/ }).fill('admin@example.com');
    const password = page.getByLabel('密码（至少 8 位）');
    await password.fill('password123');
    await password.focus();
    await page.keyboard.press('Enter');
    // Completion is real: the setup page unmounts and the app boots to home.
    await expect(page.getByText('欢迎使用 NovelWorld')).toBeHidden();
    await expect(page.getByText('开始你的旅程').first()).toBeVisible();
  });
});
