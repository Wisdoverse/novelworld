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

  test('shelf dialogs', async ({ page }) => {
    await installStubs(page);
    await page.goto('/shelf');

    await page.getByRole('button', { name: '打开共享书库' }).click();
    await expect(page.getByRole('dialog', { name: '共享书库' })).toBeVisible();
    await settleAnimations(page);
    await expectNoA11yViolations(page);
    await page.keyboard.press('Escape');

    await page.getByRole('button', { name: '导入小说' }).click();
    await expect(page.getByRole('dialog', { name: '导入小说' })).toBeVisible();
    await settleAnimations(page);
    await expectNoA11yViolations(page);
  });

  test('reader page — guided chapter with branch and chat', async ({ page }) => {
    await installStubs(page);
    await page.goto('/reader/novel-1/1');
    await expect(page.getByText('第一章 北塔来信').first()).toBeVisible();
    await expect(page.getByRole('button', { name: /收下信/ })).toBeVisible();
    await expect(page.getByRole('main')).toHaveCount(1);
    await page.waitForLoadState('networkidle');
    await expectNoA11yViolations(page);

    // Open the real ChatPanel and scan it in context.
    await page.getByRole('button', { name: /角色/ }).first().click();
    await page.getByRole('button', { name: /林晚/ }).first().click();
    await expect(page.getByRole('textbox', { name: /对 林晚 说/ })).toBeVisible();
    await expect(page.getByRole('log', { name: '已保存的对话' })).toHaveCount(1);
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
    await expect(page.getByRole('main')).toHaveCount(1);
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

test('chat announces pending/error/retry separately from committed messages', async ({ page }) => {
  await installStubs(page);
  let release!: () => void;
  const pending = new Promise<void>(resolve => { release = resolve; });
  let attempts = 0;
  const keys: string[] = [];
  await page.route('**/api/chat/*/stream', async route => {
    const turnId = route.request().headers()['idempotency-key'];
    keys.push(turnId);
    if (attempts++ === 0) {
      await pending;
      await route.fulfill({ status: 400, contentType: 'application/json', body: JSON.stringify({ error: { code: 'invalid_request', message: '暂时无法生成回复' } }) });
    } else {
      await route.fulfill({ status: 200, contentType: 'text/event-stream', body:
        'event: delta\ndata: {"content":"已保存的测试回复"}\n\n'
        + `event: done\ndata: {"turn_id":"${turnId}","committed":true,"replayed":false}\n\n` });
    }
  });
  await page.goto('/characters/novel-1');
  await page.getByRole('button', { name: /对话/ }).first().click();
  const input = page.getByRole('textbox', { name: /对 林晚 说/ });
  const log = page.getByRole('log', { name: '已保存的对话' });
  await input.fill('尚未提交的消息');
  await input.press('Enter');
  await expect(page.getByRole('status', { name: '对话状态' })).toContainText('尚未确认保存');
  await expect(log).not.toContainText('尚未提交的消息');
  release();
  await expect(page.getByRole('alert')).toContainText('暂时无法生成回复');
  await expect(log).not.toContainText('尚未提交的消息');
  await page.getByRole('button', { name: '重试', exact: true }).click();
  await expect(log).toContainText('已保存的测试回复');
  await expect(log).toContainText('尚未提交的消息');
  await expect(page.getByRole('status', { name: '对话状态' })).toBeEmpty();
  expect(keys).toHaveLength(2);
  expect(keys[0]).toBe(keys[1]);
  await expectNoA11yViolations(page);
});

test('branch and world outcomes have named committed logs and pending status', async ({ page }) => {
  await installStubs(page);
  let release!: () => void;
  const pending = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/api/narrative/choose', async route => {
    await pending;
    await route.fallback();
  });
  await page.goto('/reader/novel-1/1');
  await page.getByRole('button', { name: /收下信/ }).click();
  await expect(page.getByRole('status', { name: '分支状态' })).toContainText('尚未确认保存');
  await expect(page.getByRole('log', { name: '已保存的分支结果' })).toBeEmpty();
  release();
  await expect(page.getByRole('log', { name: '已保存的分支结果' })).toContainText('你的行动改变了后续故事');
  await expect(page.getByRole('group', { name: '阅读版本' })).toBeVisible();

  await installStubs(page, { openWorld: true });
  const worldPending = new Promise<void>(resolve => { release = resolve; });
  await page.route('**/api/narrative/*/world/turns', async route => {
    await worldPending;
    await route.fallback();
  });
  await page.reload();
  await page.getByRole('textbox', { name: '你的意图' }).fill('沿山路下行');
  await page.getByRole('button', { name: '执行行动', exact: true }).click();
  await expect(page.getByRole('status', { name: '世界行动状态' })).toContainText('正在确认世界行动');
  release();
  await expect(page.getByRole('log', { name: '旅程时间线' })).toContainText('回合 2');
  await expect(page.getByRole('status', { name: '世界行动状态' })).toBeEmpty();
  await expectNoA11yViolations(page);
});
