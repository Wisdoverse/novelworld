import { test, expect } from '@playwright/test';
import { installStubs } from './stubs';
import { expectNoA11yViolations, settleAnimations } from './helpers';

// The critical journey (issue #165 list) scanned in a real Chromium with the
// FULL WCAG 2.2 AA rule set — color-contrast from real styles, page-level
// rules on the real index.html, real widgets (no component mocks).

test.describe('critical journey — full axe rule set', () => {
  test('home page', async ({ page }) => {
    await installStubs(page);
    await page.goto('/');
    await page.waitForLoadState('networkidle');
    await expectNoA11yViolations(page);
  });

  test('login page', async ({ page }) => {
    await installStubs(page);
    await page.goto('/login');
    await page.waitForLoadState('networkidle');
    await expect(page.getByRole('textbox').first()).toBeVisible();
    await expectNoA11yViolations(page);
  });

  test('shelf page with a ready novel', async ({ page }) => {
    await installStubs(page);
    await page.goto('/shelf');
    await expect(page.getByText('星海拾遗').first()).toBeVisible();
    await page.waitForLoadState('networkidle');
    await expectNoA11yViolations(page);
  });

  test('reader page — guided chapter with branch and chat', async ({ page }) => {
    await installStubs(page);
    await page.goto('/reader/novel-1/1');
    await expect(page.getByText('第一章 北塔来信').first()).toBeVisible();
    await expect(page.getByRole('button', { name: /收下信/ })).toBeVisible();
    await page.waitForLoadState('networkidle');
    await expectNoA11yViolations(page);

    // Open the real ChatPanel and scan it in context.
    await page.getByRole('button', { name: /角色/ }).first().click();
    await page.getByRole('button', { name: /林晚/ }).first().click();
    await expect(page.getByRole('textbox', { name: /对 林晚 说/ })).toBeVisible();
    await settleAnimations(page);
    await expectNoA11yViolations(page);
  });

  test('reader page — open world dashboard and action form', async ({ page }) => {
    await installStubs(page, { openWorld: true });
    await page.goto('/reader/novel-1/1');
    await expect(page.getByText(/的开放世界/).first()).toBeVisible();
    await page.waitForLoadState('networkidle');
    await expectNoA11yViolations(page);
  });

  test('reader page — player entry required', async ({ page }) => {
    await installStubs(page, { entryRequired: true });
    await page.goto('/reader/novel-1/1');
    await expect(page.getByText('第一章 北塔来信').first()).toBeVisible();
    // Gate the scan on the form itself, not just the chapter: the scan must
    // never run on the pre-data frame.
    await expect(page.getByText('创建你的原创角色').first()).toBeVisible();
    await page.waitForLoadState('networkidle');
    await expectNoA11yViolations(page);
  });

  test('characters page', async ({ page }) => {
    await installStubs(page);
    await page.goto('/characters/novel-1');
    await expect(page.getByRole('button', { name: /对话/ }).first()).toBeVisible();
    await page.waitForLoadState('networkidle');
    await expectNoA11yViolations(page);
  });

  test('settings page', async ({ page }) => {
    await installStubs(page);
    await page.goto('/settings');
    await expect(page.getByRole('heading', { name: '平台模型设置' })).toBeVisible();
    await page.waitForLoadState('networkidle');
    await expectNoA11yViolations(page);
  });

  test('setup page (first-run)', async ({ page }) => {
    await installStubs(page, { setupNeeded: true });
    await page.goto('/');
    await expect(page.getByText('欢迎使用 NovelWorld').first()).toBeVisible();
    await page.waitForLoadState('networkidle');
    await expectNoA11yViolations(page);
  });
});
