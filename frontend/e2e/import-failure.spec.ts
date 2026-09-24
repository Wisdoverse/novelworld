import { test, expect } from '@playwright/test';
import { NOVEL } from './fixtures';
import { expectNoA11yViolations, settleAnimations } from './helpers';
import { installStubs } from './stubs';

test('failed imports show safe guidance and the matching next action', async ({ page }) => {
  await installStubs(page);
  await page.route('**/api/novels', async route => {
    if (route.request().method() !== 'GET') return route.fallback();
    await route.fulfill({ json: [
      { ...NOVEL, id: 'missing-source', title: '需要重新上传的小说', total_chapters: 0, status: 'error', parse_error: 'The retained source file is missing; re-upload the source' },
      { ...NOVEL, id: 'unknown-error', title: '可以重试的小说', total_chapters: 0, status: 'error', parse_error: 'private-provider-token-marker' },
    ] });
  });

  await page.goto('/shelf');
  await expect(page.getByText('解析失败：原始文件已不可用，请重新导入小说。')).toBeVisible();
  await expect(page.getByText(/解析失败：具体原因未记录/)).toBeVisible();
  await expect(page.getByText('private-provider-token-marker')).toHaveCount(0);
  await expect(page.getByRole('button', { name: '重新导入文件' })).toBeVisible();
  await expect(page.getByRole('button', { name: '重试解析' })).toBeVisible();
  await settleAnimations(page);
  await expectNoA11yViolations(page);
  await page.screenshot({ path: '../docs/evidence/import-failure-guidance.png' });

  await page.getByRole('button', { name: '重新导入文件' }).click();
  await expect(page.getByRole('dialog', { name: '导入小说' })).toBeVisible();
});
