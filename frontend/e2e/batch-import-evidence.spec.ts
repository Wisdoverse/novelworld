import { test, expect } from '@playwright/test';
import { installStubs } from './stubs';
import { expectNoA11yViolations, settleAnimations } from './helpers';

test('captures bounded batch novel import', async ({ page }) => {
  await installStubs(page);
  await page.goto('/shelf');
  await expect(page.getByText('星海拾遗').first()).toBeVisible();
  await page.getByRole('button', { name: '导入小说' }).first().click();

  const dialog = page.getByRole('dialog', { name: '导入小说' });
  await expect(dialog).toBeVisible();
  const picker = page.waitForEvent('filechooser');
  await dialog.getByRole('button', { name: '选择 TXT、EPUB 或 PDF 文件（可多选）' }).focus();
  await page.keyboard.press('Enter');
  await (await picker).setFiles([
    {
      name: '星河彼岸.txt',
      mimeType: 'text/plain',
      buffer: Buffer.from('第一章 星光抵达港口。'.repeat(20)),
    },
    {
      name: '雾城来信.txt',
      mimeType: 'text/plain',
      buffer: Buffer.from('第一章 雾中的信使。'.repeat(20)),
    },
  ]);
  await expect(dialog.getByText('已选择 2 本小说')).toBeVisible();
  await expect(dialog.getByRole('button', { name: '导入 2 本' })).toBeVisible();
  await settleAnimations(page);
  await expectNoA11yViolations(page);
  await dialog.screenshot({ path: '../docs/evidence/batch-novel-import.png' });
});


test('submits 50 books as bounded sequential uploads', async ({ page }) => {
  await installStubs(page);
  let requests = 0;
  let active = 0;
  let maxActive = 0;
  await page.route('**/api/novels/upload/batch', async route => {
    active++;
    maxActive = Math.max(maxActive, active);
    const body = route.request().postData() ?? '';
    expect((body.match(/filename="/g) ?? []).length).toBe(5);
    const batch = requests++;
    await new Promise(resolve => setTimeout(resolve, 50));
    active--;
    await route.fulfill({ status: 202, json: { novels: Array.from({ length: 5 }, (_, index) => ({ novel_id: `accepted-${batch}-${index}`, status: 'accepted' })) } });
  });
  await page.goto('/shelf');
  await page.getByRole('button', { name: '导入小说' }).first().click();
  const dialog = page.getByRole('dialog', { name: '导入小说' });
  await dialog.locator('input[type=file]').setInputFiles(Array.from({ length: 50 }, (_, index) => ({
    name: `测试小说${index}.txt`, mimeType: 'text/plain', buffer: Buffer.from('第一章 测试故事。'.repeat(20)),
  })));
  await expect(dialog.getByText('已选择 50 本小说')).toBeVisible();
  await expectNoA11yViolations(page);
  await dialog.getByRole('button', { name: '导入 50 本' }).click();
  await expect(dialog.getByRole('button', { name: '取消' })).toBeDisabled();
  await expect(dialog).toBeHidden();
  expect(requests).toBe(10);
  expect(maxActive).toBe(1);
});
