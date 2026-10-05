import { test, expect, type Page } from '@playwright/test';
import { installStubs } from './stubs';
import { PROGRESS, SETUP_STATUS } from './fixtures';
import { expectNoA11yViolations, expectNoHorizontalOverflow, settleAnimations, tabTo } from './helpers';

// A Chinese browser must still receive English on first use.
test.use({ locale: 'zh-CN' });

async function expectUsableLanguageControl(page: Page, locale: 'en' | 'zh-CN') {
  const language = page.getByRole('combobox', { name: locale === 'en' ? 'Language' : '语言' });
  await expect(language).toHaveCount(1);
  await expect(language).toHaveValue(locale);
  const bounds = await language.boundingBox();
  expect(bounds?.width).toBeGreaterThanOrEqual(44);
  expect(bounds?.height).toBeGreaterThanOrEqual(44);
  await tabTo(page, language);
  await expect(page.locator('html')).toHaveAttribute('lang', locale);
  return language;
}

for (const locale of ['en', 'zh-CN'] as const) {
  test(`${locale} critical pages retain their content and pass accessibility`, async ({ page }, testInfo) => {
    if (locale === 'zh-CN') await page.addInitScript(() => localStorage.setItem('novelworld.ui.locale', 'zh-CN'));
    await installStubs(page);
    const pages = [
      ['/', locale === 'en' ? 'become a player.' : '成为其中的玩家。'],
      ['/login', locale === 'en' ? 'Sign in to NovelWorld' : '登录 NovelWorld'],
      ['/register', locale === 'en' ? 'Create account' : '创建账号'],
      ['/shelf', locale === 'en' ? 'My shelf' : '我的书架'],
      ['/characters/novel-1', locale === 'en' ? 'Character list' : '角色列表'],
      ['/settings', locale === 'en' ? 'Settings' : '设置'],
      ['/reader/novel-1/1', '第一章 北塔来信'],
    ];
    for (const [url, text] of pages) {
      await page.goto(url);
      await expect(page.getByText(text, { exact: true }).first()).toBeVisible();
      await expect(page.locator('html')).toHaveAttribute('lang', locale);
      if (url === '/reader/novel-1/1') {
        await expect(page.getByRole('navigation', { name: locale === 'en' ? 'Reading navigation' : '阅读导航' })).toHaveCount(1);
      }
      const language = page.getByRole('combobox', { name: locale === 'en' ? 'Language' : '语言' });
      await expect(language).toHaveCount(1);
      await expect(language).toHaveValue(locale);
      await expect(language).toBeVisible();
      await expect(language.locator('option')).toHaveText(['EN', '中文']);
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
    await expect(page.getByRole('navigation', { name: locale === 'en' ? 'Reading navigation' : '阅读导航' })).toHaveCount(0);
    await expect(page.getByText(locale === 'en' ? 'Events still pending in this scene · 1' : '当前场景尚待发生的事件 · 1')).toBeVisible();
    expect(await page.evaluate(() => {
      const narrative = document.getElementById('latest-world-narrative');
      const action = document.getElementById('world-action-form');
      const characters = [...document.querySelectorAll('h3')].find(node => node.textContent === '此刻同场的角色' || node.textContent === 'Characters here now');
      return Boolean(narrative && action && characters
        && (narrative.compareDocumentPosition(action) & Node.DOCUMENT_POSITION_FOLLOWING)
        && (action.compareDocumentPosition(characters) & Node.DOCUMENT_POSITION_FOLLOWING));
    })).toBe(true);
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

for (const locale of ['en', 'zh-CN'] as const) {
  const pages: Array<[string, string, boolean, string]> = [
    ['home', '/', false, locale === 'en' ? 'become a player.' : '成为其中的玩家。'],
    ['login', '/login', false, locale === 'en' ? 'Sign in to NovelWorld' : '登录 NovelWorld'],
    ['register', '/register', false, locale === 'en' ? 'Create account' : '创建账号'],
    ['shelf', '/shelf', false, locale === 'en' ? 'My shelf' : '我的书架'],
    ['reader', '/reader/novel-1/1', false, locale === 'en' ? 'What will you do next?' : '你接下来做什么？'],
    ['characters', '/characters/novel-1', false, locale === 'en' ? 'Character list' : '角色列表'],
    ['settings', '/settings', false, locale === 'en' ? 'Settings' : '设置'],
    ['setup', '/', true, locale === 'en' ? 'Welcome to NovelWorld' : '欢迎使用 NovelWorld'],
  ];
  for (const [label, path, setupNeeded, readyText] of pages) {
    test(`${locale} language selector reflows on ${label} at 320px`, async ({ page }, testInfo) => {
      await installStubs(page, {
        authenticated: !['home', 'login', 'register'].includes(label),
        setupNeeded,
        openWorld: label === 'reader',
      });
      await page.setViewportSize({ width: 320, height: 720 });
      await page.goto(path);
      const labelName = locale === 'en' ? 'Language' : '语言';
      const language = page.getByRole('combobox', { name: /^(Language|语言)$/ });
      await language.waitFor();
      await page.waitForLoadState('networkidle');
      const writes: string[] = [];
      page.on('request', request => {
        const pathname = new URL(request.url()).pathname;
        if (pathname.startsWith('/api/') && !['GET', 'HEAD', 'OPTIONS'].includes(request.method())) {
          writes.push(`${request.method()} ${pathname}`);
        }
      });
      await language.selectOption(locale);
      await expect(page.getByRole('combobox', { name: labelName })).toHaveCount(1);
      await expect(language).toHaveValue(locale);
      await expect(page.locator('html')).toHaveAttribute('lang', locale);
      await expect(page.getByText(readyText, { exact: true }).first()).toBeVisible();
      expect(await language.evaluate(element => element.closest('header, section, aside') !== null)).toBe(true);
      await expectUsableLanguageControl(page, locale);
      await expectNoHorizontalOverflow(page);
      expect(writes).toEqual([]);
      if (label === 'reader') {
        await expect(page.getByRole('navigation', { name: locale === 'en' ? 'Reading navigation' : '阅读导航' })).toHaveCount(0);
      }
      if (label === 'home') {
        await expect(page.getByRole('button', { name: locale === 'en' ? 'Sign in' : '登录', exact: true })).toBeVisible();
      }
      if (['home', 'login', 'reader', 'setup'].includes(label)) {
        await page.screenshot({ path: testInfo.outputPath(`${label}-${locale}-320.png`) });
      }
    });
  }
}

test('keyboard selection persists across routes and reloads without changing source content', async ({ page }) => {
  await installStubs(page, { authenticated: false });
  await page.goto('/login');
  await page.getByRole('textbox').first().fill('reader@example.com');
  await page.getByRole('textbox').nth(1).fill('secret-pass');
  const writes: string[] = [];
  page.on('request', request => {
    if (request.method() === 'POST') writes.push(new URL(request.url()).pathname);
  });
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
  await expect(page).toHaveURL(/\/login$/);
  expect(writes).toEqual([]);
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

for (const locale of ['en', 'zh-CN'] as const) {
  const errorViews = [
    ['configuration', '/login', '**/api/setup/status', 'Cannot check service configuration', '无法检查服务配置'],
    ['session', '/', '**/api/auth/me', 'Cannot verify your session right now', '暂时无法确认登录状态'],
    ['reader progress', '/reader/novel-1/1', '**/api/progress/novel-1', 'Cannot open this chapter right now', '暂时无法打开本章'],
    ['reader chapter', '/reader/novel-1/1', '**/api/novels/novel-1/chapters/1', 'Cannot load this chapter right now', '暂时无法加载章节'],
  ];
  for (const [label, path, endpoint, englishHeading, chineseHeading] of errorViews) {
    test(`${locale} language selector remains in the ${label} error card at 320px`, async ({ page }, testInfo) => {
      if (locale === 'zh-CN') await page.addInitScript(() => localStorage.setItem('novelworld.ui.locale', 'zh-CN'));
      await installStubs(page);
      await page.route(endpoint, route => route.fulfill({ status: 503, body: '{}' }));
      await page.setViewportSize({ width: 320, height: 720 });
      await page.goto(path);
      await expect(page.getByRole('heading', { name: locale === 'en' ? englishHeading : chineseHeading })).toBeVisible();
      const language = await expectUsableLanguageControl(page, locale);
      expect(await language.evaluate(element => element.closest('.surface-card') !== null)).toBe(true);
      await expectNoHorizontalOverflow(page);
      await expectNoA11yViolations(page);
      await page.screenshot({ path: testInfo.outputPath('error.png') });
    });
  }

  for (const reader of [false, true]) {
    test(`${locale} language selector remains available while ${reader ? 'reader progress' : 'the app'} loads at 320px`, async ({ page }, testInfo) => {
      if (locale === 'zh-CN') await page.addInitScript(() => localStorage.setItem('novelworld.ui.locale', 'zh-CN'));
      await installStubs(page);
      let releaseRequest!: () => void;
      let markRequestSeen!: () => void;
      const requestSeen = new Promise<void>(resolve => { markRequestSeen = resolve; });
      const holdRequest = new Promise<void>(resolve => { releaseRequest = resolve; });
      await page.route(reader ? '**/api/progress/novel-1' : '**/api/setup/status', async route => {
        markRequestSeen();
        await holdRequest;
        await route.fulfill({ json: reader ? PROGRESS : SETUP_STATUS });
      });
      await page.setViewportSize({ width: 320, height: 720 });
      await page.goto(reader ? '/reader/novel-1/1' : '/');
      await requestSeen;
      try {
        if (reader) await expect(page.getByLabel(locale === 'en' ? 'Restoring reading progress' : '正在恢复阅读进度')).toBeVisible();
        const language = await expectUsableLanguageControl(page, locale);
        expect(await language.evaluate(element => element.closest('main') !== null)).toBe(true);
        await expectNoHorizontalOverflow(page);
        await expectNoA11yViolations(page);
        await page.screenshot({ path: testInfo.outputPath('loading.png') });
      } finally {
        releaseRequest();
      }
    });
  }
}
