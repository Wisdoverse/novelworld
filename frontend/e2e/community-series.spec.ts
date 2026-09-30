import { test, expect } from '@playwright/test';
import { installStubs } from './stubs';
import { NOVEL } from './fixtures';
import { expectNoA11yViolations, settleAnimations } from './helpers';

test('community suggestion requires confirmation; contribution is opt-in and reversible', async ({ page }) => {
  await installStubs(page);
  const source = { ...NOVEL, id: 'source-book', title: '系列来源书' };
  const series = {
    id: 'series-1', name: '读者系列', revision: 1, background: null,
    source_novel_id: source.id, source_template: null, created_at: '2026-09-30T00:00:00Z',
  };
  let linked = false;
  let contribution = false;
  let associationCalls = 0;
  let communityCalls = 0;
  let providerCalls = 0;
  const consentWrites: boolean[] = [];

  await page.route('**/api/novels', route => route.fulfill({ json: [NOVEL, source] }));
  await page.route('**/api/novels/world-series', route => route.fulfill({ json: [series] }));
  await page.route('**/api/novels/novel-1/world-series', async route => {
    if (route.request().method() === 'PUT') {
      expect(route.request().postDataJSON()).toEqual({ series_id: series.id });
      associationCalls += 1;
      linked = true;
    }
    await route.fulfill({ json: linked ? series : null });
  });
  await page.route('**/api/novels/novel-1/world-series/community-suggestion', async route => {
    communityCalls += 1;
    await route.fulfill({ json: {
      status: 'suggested', method: 'community', reason: 'community_consensus', cached: false,
      suggestion: { series_id: series.id, source_novel_id: source.id, name: series.name,
        book: { title: source.title, author: null, genre: null } },
    } });
  });
  await page.route(/\/api\/novels\/[^/]+\/world-series\/suggestion(?:\/deepseek)?$/, async route => {
    providerCalls += 1;
    await route.fulfill({ status: 500, json: {} });
  });
  await page.route('**/api/novels/world-series/series-1/contribution', async route => {
    if (route.request().method() === 'PUT') {
      contribution = route.request().postDataJSON().enabled;
      consentWrites.push(contribution);
    }
    await route.fulfill({ json: { enabled: contribution } });
  });

  await page.goto('/shelf');
  await page.getByRole('button', { name: '系列管理', exact: true }).first().click();
  expect(communityCalls).toBe(0);
  await page.getByRole('button', { name: '参考读者关联' }).click();
  await expect(page.getByText(/这里只建议分组，不证明共享世界背景相同/)).toBeVisible();
  expect(associationCalls).toBe(0);
  await page.getByRole('button', { name: '选择此系列建议' }).click();
  expect(associationCalls).toBe(0);
  await settleAnimations(page);
  await expectNoA11yViolations(page);
  await page.getByRole('button', { name: '确认关联当前书' }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(associationCalls).toBe(1);

  await page.getByRole('button', { name: '系列管理', exact: true }).first().click();
  await expect(page.getByRole('heading', { name: '系列管理' })).toBeVisible();
  const consent = page.getByRole('checkbox', { name: '允许将本系列的作品关联用于读者推荐' });
  await expect(consent).not.toBeChecked();
  await expect(consent).toBeEnabled();
  expect(consentWrites).toEqual([]);
  await consent.click();
  await expect(consent).toBeChecked();
  await expect(consent).toBeEnabled();
  await consent.click();
  await expect(consent).not.toBeChecked();
  expect(consentWrites).toEqual([true, false]);
  expect(providerCalls).toBe(0);
  expect(communityCalls).toBe(1);
  await settleAnimations(page);
  await expectNoA11yViolations(page);
});
