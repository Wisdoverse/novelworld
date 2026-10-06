import { test, expect } from '@playwright/test';
import { installStubs } from './stubs';
import { expectNoA11yViolations } from './helpers';

const novelId = 'a3924eb2-a039-4fea-a696-c14dc25e2894';
const fromId = 'e9e895cb-e34d-4a1c-9780-c0b82d5966a3';
const toId = 'b2e2f767-9299-4cdb-8a08-5da7c35a9b6d';

for (const locale of ['en', 'zh-CN'] as const) {
  test(`${locale} characters page shows progress-bounded source citations`, async ({ page }) => {
    await page.addInitScript((selectedLocale) => {
      localStorage.setItem('novelworld.ui.locale', selectedLocale);
    }, locale);
    await installStubs(page);

    // These last-registered routes override only this test's three API reads.
    await page.route(`**/api/progress/${novelId}`, route => route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        user_id: 'user-1',
        novel_id: novelId,
        current_chapter: 2,
        reader_identity_type: 'self',
      }),
    }));
    await page.route(`**/api/novels/${novelId}/characters`, route => route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify([
        { id: fromId, novel_id: novelId, name: '林晚', first_appearance_chapter: 1, persona_source_chapter_high_water: 2 },
        { id: toId, novel_id: novelId, name: '老船长', first_appearance_chapter: 1, persona_source_chapter_high_water: 2 },
      ]),
    }));
    await page.route(`**/api/novels/${novelId}/relationships/source-v1`, route => route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        novel_id: novelId,
        model_version: 1,
        checkpoint_chapter: 2,
        characters: [
          { id: fromId, name: '林晚' },
          { id: toId, name: '老船长' },
        ],
        relationships: [{
          id: 'relation:source:1',
          from_character_id: fromId,
          to_character_id: toId,
          kind: '互相守望',
          description: '两人共同守护北塔与海港。',
          source_citations: [{ chapter_number: 2, excerpt: '船长把灯递给林晚。' }],
        }],
      }),
    }));

    await page.goto(`/characters/${novelId}`);
    await expect(page.getByRole('heading', {
      name: locale === 'en' ? 'Source relationships' : '原著人物关系',
    })).toBeVisible();
    await expect(page.getByText('两人共同守护北塔与海港。')).toBeVisible();
    const citation = page.getByRole('link', { name: locale === 'en' ? 'Chapter 2' : '第 2 章' });
    await expect(citation).toHaveAttribute('href', `/reader/${novelId}/2`);
    await expect(page.getByText('船长把灯递给林晚。')).toHaveAttribute('lang', 'zh-CN');
    await expectNoA11yViolations(page);
  });
}
