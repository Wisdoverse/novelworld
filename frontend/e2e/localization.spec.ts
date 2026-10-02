import { test, expect } from '@playwright/test';
import { installStubs } from './stubs';
import { expectNoA11yViolations, settleAnimations } from './helpers';

// A Chinese browser must still receive English on first use.
test.use({ locale: 'zh-CN' });

for (const locale of ['en', 'zh-CN'] as const) {
  test(`${locale} critical pages retain their content and pass accessibility`, async ({ page }, testInfo) => {
    if (locale === 'zh-CN') await page.addInitScript(() => localStorage.setItem('novelworld.ui.locale', 'zh-CN'));
    await installStubs(page);
    const pages = [
      ['/', locale === 'en' ? 'become a player.' : '成为其中的玩家。'],
      ['/login', locale === 'en' ? 'Sign in to NovelWorld' : '登录 NovelWorld'],
      ['/shelf', locale === 'en' ? 'My shelf' : '我的书架'],
      ['/characters/novel-1', locale === 'en' ? 'Character list' : '角色列表'],
      ['/settings', locale === 'en' ? 'Settings' : '设置'],
      ['/reader/novel-1/1', '第一章 北塔来信'],
    ];
    for (const [url, text] of pages) {
      await page.goto(url);
      await expect(page.getByText(text, { exact: true }).first()).toBeVisible();
      await expect(page.locator('html')).toHaveAttribute('lang', locale);
      await expect(page.getByRole('combobox', { name: locale === 'en' ? 'Language' : '语言' })).toHaveValue(locale);
      await expect(page.getByRole('region', { name: locale === 'en' ? /Notifications/ : /通知/ })).toHaveCount(1);
      await settleAnimations(page);
      await expectNoA11yViolations(page);
      if (locale === 'en' && ['/', '/login', '/settings'].includes(url)) {
        await page.screenshot({ path: testInfo.outputPath(url === '/' ? 'home.png' : `${url.slice(1)}.png`) });
      }
    }
    await expect(page.getByRole('button', { name: locale === 'en' ? 'Translate into Chinese' : '翻译成中文' })).toHaveCount(0);
    await expect(page.getByRole('button', { name: /收下信/ })).toBeVisible();
    await expect(page.locator('.reader-content')).toHaveAttribute('lang', 'zh-CN');
    await expect(page.getByText('收下信，答应出海', { exact: true })).toHaveAttribute('lang', 'zh-CN');

    await installStubs(page, { openWorld: true });
    await page.goto('/reader/novel-1/1');
    await expect(page.getByRole('heading', { name: locale === 'en' ? 'What will you do next?' : '你接下来做什么？' })).toBeVisible();
    await expect(page.getByRole('button', { name: locale === 'en' ? 'Execute action' : '执行行动', exact: true })).toBeVisible();
    await expect(page.locator('#latest-world-narrative')).toHaveAttribute('lang', 'zh-CN');
    await expectNoA11yViolations(page);
    if (locale === 'en') await page.screenshot({ path: testInfo.outputPath('reader.png') });

    await installStubs(page, { setupNeeded: true, authenticated: false });
    await page.goto('/');
    await expect(page.getByRole('heading', { name: locale === 'en' ? 'Welcome to NovelWorld' : '欢迎使用 NovelWorld' })).toBeVisible();
    await expect(page.getByRole('combobox', { name: locale === 'en' ? 'Language' : '语言' })).toBeVisible();
    await expectNoA11yViolations(page);
    if (locale === 'en') await page.screenshot({ path: testInfo.outputPath('setup.png') });
  });
}

test('keyboard selection persists across routes and reloads without changing source content', async ({ page }) => {
  await installStubs(page, { authenticated: false });
  await page.goto('/login');
  await expect(page).toHaveTitle('NovelWorld — Enter a novel world');
  await expect(page.locator('meta[name="description"]')).toHaveAttribute('content', 'Import novels, meet their characters and shape your own story.');
  const language = page.getByRole('combobox', { name: 'Language' });
  await language.focus();
  await language.press('ArrowDown');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('heading', { name: '登录 NovelWorld' })).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('lang', 'zh-CN');
  await expect(page).toHaveTitle('NovelWorld — 进入小说的世界');
  await expect(page.locator('meta[name="description"]')).toHaveAttribute('content', '导入小说，与书中角色相遇，写下自己的故事。');
  await page.reload();
  await expect(page.getByRole('combobox', { name: '语言' })).toHaveValue('zh-CN');
  await page.getByRole('combobox', { name: '语言' }).selectOption('en');
  await expect(page.getByRole('heading', { name: 'Sign in to NovelWorld' })).toBeVisible();
  await expect(page).toHaveTitle('NovelWorld — Enter a novel world');
  await expect(page.locator('meta[name="description"]')).toHaveAttribute('content', 'Import novels, meet their characters and shape your own story.');
  await page.reload();
  await expect(page.getByRole('combobox', { name: 'Language' })).toHaveValue('en');
});

test('invalid saved preference falls back to English in a Chinese browser', async ({ page }) => {
  await installStubs(page, { authenticated: false });
  await page.addInitScript(() => localStorage.setItem('novelworld.ui.locale', 'invalid'));
  await page.goto('/login');
  await expect(page.getByRole('heading', { name: 'Sign in to NovelWorld' })).toBeVisible();
  await expect(page.getByRole('combobox', { name: 'Language' })).toHaveValue('en');
});

test('setup tolerates blocked locale storage and keeps the current language usable', async ({ page }) => {
  await installStubs(page, { setupNeeded: true, authenticated: false });
  await page.addInitScript(() => {
    localStorage.setItem('novelworld.ui.locale', 'invalid');
    const getItem = Storage.prototype.getItem;
    const setItem = Storage.prototype.setItem;
    Storage.prototype.getItem = function (key) {
      if (key === 'novelworld.ui.locale') throw new Error('locale storage blocked');
      return getItem.call(this, key);
    };
    Storage.prototype.setItem = function (key, value) {
      if (key === 'novelworld.ui.locale') throw new Error('locale storage blocked');
      setItem.call(this, key, value);
    };
  });
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Welcome to NovelWorld' })).toBeVisible();
  await page.getByRole('combobox', { name: 'Language' }).selectOption('zh-CN');
  await expect(page.getByRole('heading', { name: '欢迎使用 NovelWorld' })).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('lang', 'zh-CN');
  await page.reload();
  await expect(page.getByRole('heading', { name: 'Welcome to NovelWorld' })).toBeVisible();
});

test('language selector remains available on configuration errors', async ({ page }) => {
  await installStubs(page, { authenticated: false });
  await page.route('**/api/setup/status', route => route.fulfill({ status: 503, body: '{}' }));
  await page.goto('/login');
  await expect(page.getByRole('heading', { name: 'Cannot check service configuration' })).toBeVisible();
  await page.getByRole('combobox', { name: 'Language' }).selectOption('zh-CN');
  await expect(page.getByRole('heading', { name: '无法检查服务配置' })).toBeVisible();
  await expectNoA11yViolations(page);
});
