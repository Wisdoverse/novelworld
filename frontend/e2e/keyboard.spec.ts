import { test, expect, type Locator } from '@playwright/test';
import { installStubs } from './stubs';
import { tabWalk, tabTo, currentFocus, settleAnimations } from './helpers';

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
    await expect(dialog.getByRole('button', { name: '将《星海拾遗》加入书架' })).toBeVisible();
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
    await expect(dialog.getByText(/仍在解析或随后解析失败的内容/)).toBeVisible();
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

  test('reader: drawer and non-modal chat have keyboard entry, exit, and restoration', async ({ page }) => {
    await installStubs(page);
    await page.goto('/reader/novel-1/1');
    await expect(page.getByRole('main')).toBeVisible();
    const trigger = page.getByRole('button', { name: '角色', exact: true });
    await tabTo(page, trigger);
    await page.keyboard.press('Enter');
    const drawer = page.getByRole('dialog', { name: '故事角色' });
    await expectFocusWithin(drawer);
    for (const key of ['Tab', 'Shift+Tab']) {
      for (let i = 0; i < 6; i++) {
        await page.keyboard.press(key);
        await expectFocusWithin(drawer);
        expect((await currentFocus(page))?.inViewport).toBe(true);
      }
    }
    await page.keyboard.press('Escape');
    await expect(trigger).toBeFocused();
    await page.keyboard.press('Enter');
    await tabTo(page, drawer.getByRole('button', { name: /林晚/ }));
    await page.keyboard.press('Enter');
    const chat = page.getByRole('region', { name: '与 林晚 对话' });
    await expect(drawer).toBeHidden();
    await settleAnimations(page);
    await expect(chat.getByRole('button', { name: '关闭聊天' })).toBeFocused();
    const input = chat.getByRole('textbox');
    await tabTo(page, input);
    await tabTo(page, chat.getByRole('button', { name: '收起聊天窗口' }), 'Shift+Tab');
    await page.keyboard.press('Enter');
    await expect(input).toBeHidden();
    await page.keyboard.press('Enter');
    await expect(input).toBeVisible();
    await tabTo(page, input);
    // A non-modal chat must allow Tab back to the chapter toolbar.
    await tabTo(page, trigger);
    await expect(chat).toBeVisible();
    await tabTo(page, input, 'Shift+Tab');
    await page.keyboard.press('Escape');
    await expect(chat).toBeHidden();
    await expect(trigger).toBeFocused();
  });

  test('characters: closing chat restores the card trigger', async ({ page }) => {
    await installStubs(page);
    await page.goto('/characters/novel-1');
    await expect(page.getByRole('main')).toBeVisible();
    const trigger = page.getByRole('button', { name: /对话/ }).first();
    await tabTo(page, trigger);
    await page.keyboard.press('Enter');
    const close = page.getByRole('button', { name: '关闭聊天' });
    await expect(close).toBeFocused();
    await settleAnimations(page);
    await page.keyboard.press('Enter');
    await expect(close).toBeHidden();
    await expect(trigger).toBeFocused();
  });

  test('world action form submits from the keyboard', async ({ page }) => {
    await installStubs(page, { openWorld: true });
    await page.goto('/reader/novel-1/1');
    await expect(page.getByText(/的开放世界/).first()).toBeVisible();
    await page.getByRole('textbox', { name: '你的意图' }).fill('沿山路下行');
    const submit = page.getByRole('button', { name: '执行行动', exact: true });
    await tabTo(page, submit);
    await page.keyboard.press('Space');
    // The turn stub commits: the rendered narrative of the transition appears.
    await expect(page.getByRole('log', { name: '旅程时间线' })).toContainText('回合 2');
  });

  test('settings: tab walk stays in the page and every stop has a focus indicator', async ({ page }) => {
    await installStubs(page);
    await page.goto('/settings');
    await expect(page.getByRole('heading', { name: '平台模型设置' })).toBeVisible();
    await expect(page.getByText(/仍在解析或随后解析失败的内容/)).toBeVisible();
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
