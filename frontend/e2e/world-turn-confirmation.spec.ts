import { test, expect } from '@playwright/test';
import { installStubs } from './stubs';

for (const locale of ['en', 'zh-CN'] as const) test.describe(locale, () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(value => localStorage.setItem('novelworld.ui.locale', value), locale);
  });

test('a durable failed turn unlocks automatically without submitting the action again', async ({ page }, testInfo) => {
  await installStubs(page, { openWorld: true });
  const posts: Array<{ key: string; body: unknown }> = [];
  const reads: string[] = [];
  await page.route('**/api/narrative/*/world/turns', async route => {
    posts.push({ key: route.request().headers()['idempotency-key'], body: route.request().postDataJSON() });
    await route.fulfill({ status: 502, contentType: 'application/json', body: JSON.stringify({ error: { code: 'llm_error', message: 'Action generation failed' } }) });
  });
  await page.route('**/api/narrative/*/world/turns/*', async route => {
    const key = route.request().url().split('/').pop()!;
    reads.push(key);
    await route.fulfill({ contentType: 'application/json', body: JSON.stringify({ turn_id: key, status: 'failed' }) });
  });
  await page.goto('/reader/novel-1/1');
  await page.getByRole('button', { name: locale === 'en' ? 'Write freely' : '自由输入', exact: true }).click();
  await page.getByRole('textbox', { name: locale === 'en' ? 'Your intent' : '你的意图' }).fill('寻找当前场景的线索');
  await page.getByRole('button', { name: locale === 'en' ? 'Execute action' : '执行行动', exact: true }).click();
  await expect(page.getByText(locale === 'en' ? /failed before it changed your world/ : /行动未能完成，世界没有因此改变/)).toBeVisible();
  await expect(page.getByRole('button', { name: locale === 'en' ? 'Check result' : '继续确认结果' })).toHaveCount(0);
  expect(posts).toHaveLength(1);
  expect(reads.length).toBeGreaterThan(0);
  expect(reads.every(key => key === posts[0].key)).toBe(true);
  expect(await page.evaluate(() => Object.keys(sessionStorage).filter(key => key.startsWith('novelworld:pending-world-turn:')))).toEqual([]);
  await page.getByRole('textbox', { name: locale === 'en' ? 'Your intent' : '你的意图' }).fill('选择下一项行动');
  await expect(page.getByRole('button', { name: locale === 'en' ? 'Execute action' : '执行行动', exact: true })).toBeEnabled();
  expect(posts).toHaveLength(1);
  await page.locator('section[aria-labelledby="living-world-title"]').screenshot({ path: testInfo.outputPath('failed-action-confirmation.png') });
});

test('reload confirms the exact stored key with reads while a missing result remains locked', async ({ page }) => {
  await installStubs(page, { openWorld: true });
  let postedKey = '';
  let posts = 0;
  let failed = false;
  const reads: string[] = [];
  await page.route('**/api/narrative/*/world/turns', async route => {
    posts++;
    postedKey = route.request().headers()['idempotency-key'];
    await route.fulfill({ status: 502, contentType: 'application/json', body: JSON.stringify({ error: { code: 'llm_error', message: 'Action generation failed' } }) });
  });
  await page.route('**/api/narrative/*/world/turns/*', async route => {
    const key = route.request().url().split('/').pop()!;
    reads.push(key);
    await route.fulfill({ status: failed ? 200 : 404, contentType: 'application/json', body: JSON.stringify(failed ? { turn_id: key, status: 'failed' } : { error: { code: 'not_found', message: 'No confirmed result' } }) });
  });
  await page.goto('/reader/novel-1/1');
  await page.getByRole('button', { name: locale === 'en' ? 'Write freely' : '自由输入', exact: true }).click();
  await page.getByRole('textbox', { name: locale === 'en' ? 'Your intent' : '你的意图' }).fill('寻找当前场景的线索');
  await page.getByRole('button', { name: locale === 'en' ? 'Execute action' : '执行行动', exact: true }).click();
  const confirm = page.getByRole('button', { name: locale === 'en' ? 'Check result' : '继续确认结果' });
  await expect(confirm).toBeEnabled();
  await expect(page.getByRole('button', { name: locale === 'en' ? 'Execute action' : '执行行动', exact: true })).toBeDisabled();
  await confirm.click();
  await expect.poll(() => reads.length).toBeGreaterThanOrEqual(2);
  expect(posts).toBe(1);
  const original = await page.evaluate(() => Object.entries(sessionStorage).find(([key]) => key.startsWith('novelworld:pending-world-turn:'))?.[1]);
  expect(JSON.parse(original!).idempotencyKey).toBe(postedKey);
  failed = true;
  await page.reload();
  await expect(page.getByText(locale === 'en' ? /failed before it changed your world/ : /行动未能完成，世界没有因此改变/)).toBeVisible();
  expect(reads.every(key => key === postedKey)).toBe(true);
  expect(posts).toBe(1);
  expect(await page.evaluate(() => Object.keys(sessionStorage).filter(key => key.startsWith('novelworld:pending-world-turn:')))).toEqual([]);
});

});
