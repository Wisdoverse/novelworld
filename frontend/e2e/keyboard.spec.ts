import { test, expect } from '@playwright/test';
import { installStubs } from './stubs';
import { tabWalk } from './helpers';

// Keyboard operability on the critical journey: real Tab focus movement,
// visible focus indicators, Enter/Space activation, and focus containment.

test.describe('critical journey — keyboard operability', () => {
  test('login: tab walk, visible focus, Enter submits and lands on the shelf', async ({ page }) => {
    await installStubs(page);
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
    const card = page.getByRole('button', { name: /星海拾遗/ }).first();
    await card.focus();
    await page.keyboard.press('Enter');
    await expect(page).toHaveURL(/\/reader\/novel-1\/1/);
  });

  test('reader: branch choice activates with Space and renders the consequence', async ({ page }) => {
    await installStubs(page);
    await page.goto('/reader/novel-1/1');
    const choice = page.getByRole('button', { name: /收下信/ }).first();
    await choice.focus();
    await page.keyboard.press('Space');
    await expect(page.getByText('旅人收下信，约定黎明出海。').first()).toBeVisible();
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
    await expect(page.getByText('模型设置').first()).toBeVisible();
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
    const submit = page.getByRole('button', { name: /创建|进入/ }).first();
    await submit.focus();
    await page.keyboard.press('Enter');
    await expect(page.getByText('无名旅人').first()).toBeVisible();
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
    const input = page.getByRole('textbox').first();
    await input.fill('sk-test-key');
    await page.getByRole('button', { name: /下一步/ }).first().click();
    await page.getByRole('button', { name: /完成|开始使用/ }).first().click();
    await expect(page.getByText('欢迎使用 NovelWorld').first()).toBeVisible();
  });
});
