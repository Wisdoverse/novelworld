import { test, expect } from '@playwright/test';
import { installStubs } from './stubs';
import { expectNoA11yViolations, settleAnimations } from './helpers';

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
