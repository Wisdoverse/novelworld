import { test, expect } from '@playwright/test';
import { installStubs } from './stubs';
import { expectNoA11yViolations, settleAnimations } from './helpers';

test('locked rules explain reading progress and allow narrative entry without retries', async ({ page }) => {
  await installStubs(page, { entryRequired: true });
  let requests = 0;
  await page.route('**/api/narrative/novel-1/game-rules', async route => {
    requests += 1;
    await route.fulfill({
      status: 422,
      contentType: 'application/json',
      body: JSON.stringify({ error: {
        code: 'game_rules_unavailable_at_progress',
        message: 'Game rules are not yet available at current reading progress',
      } }),
    });
  });
  await page.goto('/reader/novel-1/1');
  const advanced = page.getByRole('checkbox', { name: /启用小说专属 D20/ });
  await advanced.check();
  await page.getByRole('button', { name: '生成小说专属规则' }).click();
  await expect(page.getByRole('alert')).toContainText('尚未解锁的章节');
  await expect(page.getByText('Novel service is unavailable')).toHaveCount(0);
  await settleAnimations(page);
  await expectNoA11yViolations(page);
  if (process.env.CAPTURE_D20_PROGRESS) {
    await page.locator('.fixed').evaluateAll(elements => {
      elements.forEach(element => {
        (element as HTMLElement).style.visibility = 'hidden';
      });
    });
    await page.getByRole('region', { name: '创建你的原创角色' }).screenshot({
      path: '../docs/evidence/d20-reading-progress.png',
    });
  }
  await page.getByLabel('名字').fill('燕七');
  await page.getByLabel('背景').fill('破庙里的落魄刀客');
  await page.getByLabel('能力（用逗号分隔）').fill('听风');
  await advanced.uncheck();
  await expect(page.getByRole('alert')).toHaveCount(0);
  const submitted = page.waitForRequest(req => (
    req.method() === 'PUT' && /\/api\/narrative\/novel-1\/player-entry$/.test(req.url())
  ));
  await page.getByRole('button', { name: '进入故事' }).click();
  expect((await submitted).postDataJSON().rules.mode).toBe('narrative');
  expect(requests).toBe(1);
});

test('advanced rules: generate, allocate, preview, and submit the pinned profile', async ({ page }) => {
  await installStubs(page, { entryRequired: true });
  await page.goto('/reader/novel-1/1');
  await expect(page.getByText('创建你的原创角色')).toBeVisible();

  await page.getByRole('checkbox', { name: /启用小说专属 D20/ }).check();
  await page.getByRole('button', { name: '生成小说专属规则' }).click();
  await expect(page.getByText('属性点 30 / 30')).toBeVisible();
  await expect(page.getByText('轻功')).toBeVisible();
  await expect(page.getByText('D20 · 服务器判定')).toBeVisible();

  await page.getByLabel('名字').fill('燕七');
  await page.getByLabel('背景').fill('破庙里的落魄刀客');
  await page.getByLabel('能力（用逗号分隔）').fill('听风，辨穴');
  await settleAnimations(page);
  await expectNoA11yViolations(page);

  if (process.env.CAPTURE_ADVANCED_RULES) {
    const entryForm = page.getByRole('region', { name: '创建你的原创角色' });
    await page.locator('.fixed').evaluateAll(elements => {
      elements.forEach(element => {
        (element as HTMLElement).style.visibility = 'hidden';
      });
    });
    await entryForm.scrollIntoViewIfNeeded();
    await entryForm.screenshot({ path: '../docs/evidence/advanced-rules.png' });
  }

  const request = page.waitForRequest(req => (
    req.method() === 'PUT' && /\/api\/narrative\/novel-1\/player-entry$/.test(req.url())
  ));
  await page.getByRole('button', { name: '进入故事' }).click();
  expect((await request).postDataJSON()).toMatchObject({
    name: '燕七',
    rules: {
      mode: 'advanced',
      canon_model_version: 1,
      attributes: { qinggong: 10, dongcha: 10, renmai: 10 },
    },
  });
});
