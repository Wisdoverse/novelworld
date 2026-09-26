import { test, expect } from '@playwright/test';
import { installStubs } from './stubs';
import { expectNoA11yViolations, settleAnimations } from './helpers';

test('regional coding plans show their endpoint and require a matching key', async ({ page }) => {
  await installStubs(page);
  await page.goto('/settings');
  const provider = page.getByLabel('服务商 / 地区 / 套餐');
  await provider.selectOption('zhipu_coding');
  await expect(page.getByText('端点：https://open.bigmodel.cn/api/coding/paas/v4', { exact: true })).toBeVisible();
  await expect(page.getByText(/NovelWorld 应用后端不在已核实的支持范围内/)).toBeVisible();
  await provider.selectOption('minimax_coding_cn');
  await expect(page.getByLabel('模型', { exact: true })).toHaveValue('MiniMax-M3');
  const key = page.getByLabel('平台 API Key', { exact: true });
  await key.fill('synthetic-cn-key');
  await provider.selectOption('minimax_coding_global');
  await expect(key).toHaveValue('');
  await expect(key).toHaveAttribute('required', '');
  await expect(page.getByText('端点：https://api.minimax.io/v1', { exact: true })).toBeVisible();
  await page.setViewportSize({ width: 320, height: 740 });
  await settleAnimations(page);
  await expectNoA11yViolations(page);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.screenshot({ path: '../docs/evidence/llm-provider-regions.png', fullPage: true });
});
