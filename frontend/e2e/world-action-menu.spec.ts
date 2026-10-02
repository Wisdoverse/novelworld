import { test, expect } from '@playwright/test';
import { installStubs } from './stubs';
import { expectNoA11yViolations, expectNoHorizontalOverflow, tabTo } from './helpers';

test('default scene suggestions are keyboard usable drafts on narrow screens', async ({ page }, testInfo) => {
  await installStubs(page, { openWorld: true, actionSuggestions: true });
  let turnPosts = 0;
  let classifierPosts = 0;
  page.on('request', request => {
    if (request.method() !== 'POST') return;
    if (request.url().endsWith('/world/turns')) turnPosts += 1;
    if (request.url().endsWith('/world/action-suggestion')) classifierPosts += 1;
  });
  await page.setViewportSize({ width: 320, height: 720 });
  await page.goto('/reader/novel-1/1');

  const menu = page.getByRole('group', { name: '场景建议' });
  const actionForm = page.locator('form').filter({ has: menu });
  const advanced = actionForm.locator('details');
  await expect(menu.getByRole('button')).toHaveCount(3);
  for (const number of [1, 2, 3]) await expect(menu.getByRole('button', { name: new RegExp(`^${number}\\.`) })).toBeVisible();
  await expect(page.getByRole('button', { name: '自由输入', exact: true })).toBeVisible();
  await expect(advanced).not.toHaveAttribute('open', '');
  await expectNoHorizontalOverflow(page);
  await expectNoA11yViolations(page);
  await menu.scrollIntoViewIfNeeded();
  await actionForm.screenshot({ path: testInfo.outputPath('world-action-menu-320.png') });
  await page.setViewportSize({ width: 1280, height: 800 });
  await menu.scrollIntoViewIfNeeded();
  await actionForm.screenshot({ path: testInfo.outputPath('world-action-menu-desktop.png') });

  const firstSuggestion = menu.getByRole('button').first();
  await firstSuggestion.focus();
  await page.keyboard.press('Enter');
  await expect(firstSuggestion).toHaveAttribute('aria-pressed', 'true');
  const freeInput = page.getByRole('button', { name: '自由输入', exact: true });
  await tabTo(page, freeInput);
  await page.keyboard.press('Enter');
  await expect(freeInput).toHaveAttribute('aria-pressed', 'true');
  expect(turnPosts).toBe(0);
  expect(classifierPosts).toBe(0);
});

test('scene suggestion executes the edited intent and refreshes the next turn', async ({ page }) => {
  await installStubs(page, { openWorld: true });
  const requests: Array<{ body: Record<string, unknown>; key: string | undefined }> = [];
  await page.route('**/api/narrative/*/world/turns', async route => {
    requests.push({
      body: route.request().postDataJSON(),
      key: route.request().headers()['idempotency-key'],
    });
    await route.fallback();
  });
  await page.goto('/reader/novel-1/1');
  const threadSuggestion = page.getByRole('group', { name: '场景建议' }).getByRole('button', { name: /^2\./ });
  await threadSuggestion.click();
  await expect(page.getByRole('textbox', { name: '你的意图' })).not.toHaveValue('');
  await expect(page.getByRole('button', { name: '执行行动', exact: true })).toBeEnabled();
  await expect(requests).toHaveLength(0);
  await page.getByRole('textbox', { name: '你的意图' }).fill('  查看信中的召唤  ');
  await expect(page.getByRole('button', { name: '执行行动', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: '执行行动', exact: true }).click();

  await expect(page.getByRole('log', { name: '旅程时间线' })).toContainText('回合 2');
  expect(requests).toHaveLength(1);
  expect(requests[0].body).toMatchObject({ kind: 'investigate', target_id: 'th-1', intent: '查看信中的召唤', expected_turn_number: 1 });
  expect(requests[0].key).toMatch(/^[0-9a-f-]{36}$/i);
  await expect(page.getByRole('group', { name: '场景建议' }).getByRole('button')).toHaveCount(3);
  const nextIntent = page.getByRole('textbox', { name: '你的意图' });
  await expect(nextIntent).toHaveValue('');
  await nextIntent.fill('继续观察当前场景');
  await expect(page.getByRole('button', { name: '执行行动', exact: true })).toBeEnabled();
  expect(requests).toHaveLength(1);
});

test('free input submits pursue_goal with a null target only after explicit execution', async ({ page }) => {
  await installStubs(page, { openWorld: true });
  const requests: Array<{ body: Record<string, unknown>; key: string | undefined }> = [];
  await page.route('**/api/narrative/*/world/turns', async route => {
    requests.push({
      body: route.request().postDataJSON(),
      key: route.request().headers()['idempotency-key'],
    });
    await route.fallback();
  });
  await page.goto('/reader/novel-1/1');
  const submit = page.getByRole('button', { name: '执行行动', exact: true });
  await page.getByRole('button', { name: '自由输入', exact: true }).click();
  await expect(submit).toBeDisabled();
  await expect(requests).toHaveLength(0);
  await page.getByRole('textbox', { name: '你的意图' }).fill('  整理我已知的线索  ');
  await expect(submit).toBeEnabled();
  await expect(requests).toHaveLength(0);
  await submit.click();

  await expect(page.getByRole('log', { name: '旅程时间线' })).toContainText('回合 2');
  expect(requests).toHaveLength(1);
  expect(requests[0].body).toMatchObject({
    kind: 'pursue_goal', target_id: null, intent: '整理我已知的线索', expected_turn_number: 1,
  });
  expect(requests[0].key).toMatch(/^[0-9a-f-]{36}$/i);
});

// Exercise the retained Chinese UI without changing the application's English default.
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('novelworld.ui.locale', 'zh-CN'));
});
