import { test, expect } from '@playwright/test';
import { installStubs } from './stubs';
import { expectNoA11yViolations, settleAnimations } from './helpers';

// H4 identity boundary: a character-identity reader keeps conversation and
// branch choices, sees NO player-entry or open-world agency, and scans clean
// under the full rule set.

test.describe('character-identity boundary (SPEC §8.2)', () => {
  test('reader page: chat + branch only, no open-world or player-entry agency', async ({ page }) => {
    await installStubs(page, { characterIdentity: true });
    await page.goto('/reader/novel-1/1');
    await expect(page.getByText('第一章 北塔来信').first()).toBeVisible();
    // Branch choices remain available in character mode.
    await expect(page.getByRole('button', { name: /收下信/ })).toBeVisible();
    // Negative assertions: the open-world and player-entry agency MUST be absent.
    await expect(page.getByRole('button', { name: /进入开放世界/ })).toHaveCount(0);
    await expect(page.getByText('创建你的原创角色')).toHaveCount(0);
    await page.waitForLoadState('networkidle');
    await expectNoA11yViolations(page);

    // In-character chat still works and scans clean.
    await page.getByRole('button', { name: /角色/ }).first().click();
    await page.getByRole('button', { name: /林晚/ }).first().click();
    await expect(page.getByRole('textbox', { name: /对 林晚 说/ })).toBeVisible();
    await settleAnimations(page);
    await expectNoA11yViolations(page);
  });

  test('branch choice commits in character mode', async ({ page }) => {
    await installStubs(page, { characterIdentity: true });
    await page.goto('/reader/novel-1/1');
    const choice = page.getByRole('button', { name: /收下信/ }).first();
    await choice.focus();
    await page.keyboard.press('Space');
    await expect(page.getByText('旅人收下信，约定黎明出海。').first()).toBeVisible();
  });

  test('character mode never opens the world dashboard', async ({ page }) => {
    await installStubs(page, { characterIdentity: true, openWorld: true });
    await page.goto('/reader/novel-1/1');
    // Even with an open-world view behind the stub, character identity must
    // not render it (the client gates on isSelfMode + player entry).
    await expect(page.getByText(/的开放世界/)).toHaveCount(0);
  });
});
