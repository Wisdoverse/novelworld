import { test, expect } from '@playwright/test';
import { installStubs } from './stubs';
import { GAME_RULE_TEMPLATE } from './fixtures';
import { expectNoA11yViolations, settleAnimations } from './helpers';

test('create shared background first, then explicitly generate source D20 rules', async ({ page }) => {
  await installStubs(page);
  const pending = {
    id: 'series-1', name: '星海系列', background: '读者确认的共同背景',
    revision: 1, source_novel_id: 'novel-1', source_template: null,
    created_at: '2026-09-28T00:00:00Z',
  };
  const ready = {
    ...pending,
    source_template: { ...GAME_RULE_TEMPLATE, prompt_version: 'novel-game-rules-v2' },
  };
  let current: typeof pending | typeof ready | null = null;
  let generationCalls = 0;

  await page.route('**/api/novels/world-series', async route => {
    const request = route.request();
    if (request.method() === 'POST') {
      expect(request.postDataJSON()).toMatchObject({
        name: pending.name, background: pending.background, source_novel_id: pending.source_novel_id,
      });
      current = pending;
      await route.fulfill({ status: 201, contentType: 'application/json', body: JSON.stringify(current) });
    } else {
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(current ? [current] : []) });
    }
  });
  await page.route('**/api/novels/novel-1/world-series', async route => {
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(current) });
  });
  await page.route('**/api/narrative/novel-1/game-rules', async route => {
    generationCalls += 1;
    current = ready;
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        ...ready.source_template,
        prompt_version: 'series-game-rules-v1',
        series: {
          binding: { series_id: ready.id, revision: 1 },
          target_novel_id: 'novel-1', name: ready.name, background: ready.background,
        },
      }),
    });
  });

  await page.goto('/shelf');
  await page.getByRole('button', { name: '识别同系列 / 共享世界背景' }).click();
  await page.getByRole('button', { name: '创建系列', exact: true }).click();
  await page.getByLabel('系列名称').fill(pending.name);
  await page.getByLabel('共享世界背景（最多 2000 字）').fill(pending.background);
  await page.getByLabel('世界观及未来 D20 规则来源书').selectOption('novel-1');
  await page.getByRole('button', { name: '创建系列并关联来源书与当前书' }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(generationCalls).toBe(0);

  await page.getByRole('button', { name: '识别同系列 / 共享世界背景' }).click();
  await expect(page.getByText('此系列的 D20 基础规则尚未生成。纯叙事模式可先使用共享背景。')).toBeVisible();
  await settleAnimations(page);
  await expectNoA11yViolations(page);
  if (process.env.CAPTURE_DEFERRED_SERIES) {
    await page.getByRole('dialog').screenshot({ path: '../docs/evidence/deferred-series-rules.png' });
  }
  expect(generationCalls).toBe(0);
  await page.getByRole('button', { name: '生成来源书 D20 基础规则' }).click();
  await expect(page.getByText('D20 基础规则已从来源书固定到系列。')).toBeVisible();
  expect(generationCalls).toBe(1);
});
