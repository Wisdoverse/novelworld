import { test, expect } from '@playwright/test';
import { installStubs } from './stubs';
import { GAME_RULE_TEMPLATE, NOVEL } from './fixtures';
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
  await page.getByLabel('系列来源书（未来 D20 规则来源）').selectOption('novel-1');
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

test('recognize a second book, require confirmation, then share the confirmed background', async ({ page }) => {
  await installStubs(page);
  const books = [NOVEL, { ...NOVEL, id: 'novel-2', title: '星海归途' }];
  let series: {
    id: string; name: string; background: string | null; revision: number;
    source_novel_id: string; source_template: null; created_at: string;
  } | null = null;
  const bindings: Record<string, string | null> = { 'novel-1': null, 'novel-2': null };
  let associationCalls = 0;
  let generationCalls = 0;
  let draftCalls = 0;

  await page.route('**/api/novels', async route => {
    if (route.request().method() !== 'GET') return route.fallback();
    return route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(books) });
  });
  await page.route(/\/api\/novels\/novel-[12]\/world-series$/, async route => {
    const novelId = new URL(route.request().url()).pathname.split('/')[3];
    if (route.request().method() === 'PUT') {
      associationCalls += 1;
      bindings[novelId] = route.request().postDataJSON().series_id;
    }
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(bindings[novelId] ? series : null) });
  });
  await page.route('**/api/novels/world-series', async route => {
    if (route.request().method() === 'POST') {
      expect(route.request().postDataJSON()).toMatchObject({
        name: '星海系列', background: null, source_novel_id: 'novel-1', canon_model_version: 1,
      });
      series = {
        id: 'series-1', name: '星海系列', background: null, revision: 1,
        source_novel_id: 'novel-1', source_template: null, created_at: '2026-09-28T00:00:00Z',
      };
      bindings['novel-1'] = series.id;
      await route.fulfill({ status: 201, contentType: 'application/json', body: JSON.stringify(series) });
    } else {
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(series ? [series] : []) });
    }
  });
  await page.route('**/api/novels/novel-2/world-series/suggestion', async route => {
    await route.fulfill({
      status: 200, contentType: 'application/json',
      body: JSON.stringify({
        status: 'suggested', method: 'laya', reason: 'suggested', cached: false,
        suggestion: {
          series_id: 'series-1', source_novel_id: 'novel-1', name: '星海系列',
          book: { title: NOVEL.title, author: NOVEL.author, genre: NOVEL.genre },
        },
      }),
    });
  });
  const extractedBackground = '世界背景：星海诸城共享航道。\n人物关系：甲与乙：伙伴';
  await page.route('**/api/novels/novel-1/world-series/background-draft', async route => {
    draftCalls += 1;
    await route.fulfill({
      status: 200, contentType: 'application/json',
      body: JSON.stringify({ source_novel_id: 'novel-1', canon_model_version: 1, background: extractedBackground }),
    });
  });
  await page.route('**/api/novels/world-series/series-1/background', async route => {
    expect(route.request().postDataJSON()).toEqual({ background: extractedBackground });
    expect(series).not.toBeNull();
    series = { ...series!, background: extractedBackground };
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(series) });
  });
  await page.route('**/api/narrative/novel-1/game-rules', async route => {
    generationCalls += 1;
    await route.fulfill({ status: 500, body: '{}' });
  });

  await page.goto('/shelf');
  await expect(page.getByRole('button', { name: '识别同系列 / 共享世界背景' })).toHaveCount(2);
  await page.getByRole('button', { name: '识别同系列 / 共享世界背景' }).first().click();
  await page.getByRole('button', { name: '创建系列', exact: true }).click();
  await page.getByLabel('系列名称').fill('星海系列');
  await page.getByLabel('系列来源书（未来 D20 规则来源）').selectOption('novel-1');
  expect(draftCalls).toBe(0);
  await page.getByRole('button', { name: '查看来源书的全书背景建议（可能含后文）' }).click();
  await expect(page.getByText('来源书解析建议（世界设定与人物关系）')).toBeVisible();
  expect(draftCalls).toBe(1);
  await page.getByRole('button', { name: '创建系列并关联来源书与当前书' }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(bindings['novel-1']).toBe('series-1');
  expect(associationCalls).toBe(0);

  await page.getByRole('button', { name: '识别同系列 / 共享世界背景' }).nth(1).click();
  await page.getByRole('button', { name: '识别同系列', exact: true }).click();
  await expect(page.getByRole('button', { name: '选择此系列建议' })).toBeVisible();
  expect(bindings['novel-2']).toBeNull();
  await page.getByRole('button', { name: '选择此系列建议' }).click();
  expect(bindings['novel-2']).toBeNull();
  await page.getByRole('button', { name: '确认关联当前书' }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(bindings['novel-2']).toBe('series-1');
  expect(associationCalls).toBe(1);

  await page.getByRole('button', { name: '识别同系列 / 共享世界背景' }).first().click();
  await expect(page.getByText(/当前系列仅用于分组/)).toBeVisible();
  await expect(page.getByText('D20 基础规则已从来源书固定到系列。')).toHaveCount(0);
  expect(draftCalls).toBe(1);
  await page.getByRole('button', { name: '查看来源书的全书背景建议（可能含后文）' }).click();
  await page.getByRole('button', { name: '填入原著背景建议' }).click();
  expect(draftCalls).toBe(2);
  await expect(page.getByLabel('共享世界背景（最多 2000 字）')).toHaveValue(extractedBackground);
  await page.getByRole('button', { name: '确认共享背景' }).click();
  await expect(page.getByText('此系列的 D20 基础规则尚未生成。纯叙事模式可先使用共享背景。')).toBeVisible();
  await page.getByRole('button', { name: '完成' }).click();
  await page.getByRole('button', { name: '识别同系列 / 共享世界背景' }).nth(1).click();
  await expect(page.getByText('当前系列：星海系列（背景共享；角色和进度独立）。')).toBeVisible();
  await expect(page.getByText('此系列的 D20 基础规则尚未生成。纯叙事模式可先使用共享背景。')).toBeVisible();
  expect(generationCalls).toBe(0);
});
