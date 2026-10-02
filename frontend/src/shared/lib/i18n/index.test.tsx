import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { en, zhCN } from './messages';

beforeEach(() => {
  localStorage.clear();
  vi.resetModules();
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('UI locale', () => {
  it('has complete Chinese resources with the same interpolation parameters', () => {
    expect(Object.keys(zhCN).sort()).toEqual(Object.keys(en).sort());
    for (const key of Object.keys(en) as (keyof typeof en)[]) {
      const parameters = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map(match => match[1]).sort();
      expect(parameters(zhCN[key]), key).toEqual(parameters(en[key]));
      expect(en[key], key).not.toMatch(/[\u3400-\u9fff]/);
    }
  });

  it('defaults to English independently of browser language', async () => {
    vi.spyOn(navigator, 'language', 'get').mockReturnValue('zh-CN');
    const { translate } = await import('./index');
    expect(translate('Retry')).toBe('Retry');
  });

  it.each(['fr', '', '{"locale":"zh-CN"}'])('ignores invalid stored preference %s', async value => {
    localStorage.setItem('novelworld.ui.locale', value);
    const { translate } = await import('./index');
    expect(translate('Retry')).toBe('Retry');
  });

  it('restores an explicit Chinese preference and interpolates content unchanged', async () => {
    localStorage.setItem('novelworld.ui.locale', 'zh-CN');
    const { translate } = await import('./index');
    expect(translate('Chapter {p0}', { p0: 3 })).toBe('第 3 章');
    expect(translate('Talk to {p0}', { p0: '林晚' })).toBe('与林晚交谈');
  });

  it('persists a labelled selector choice and updates rendered text', async () => {
    const { LanguageSwitcher } = await import('@/shared/ui/LanguageSwitcher');
    render(<LanguageSwitcher />);
    fireEvent.change(screen.getByRole('combobox', { name: 'Language' }), { target: { value: 'zh-CN' } });
    expect(screen.getByRole('combobox', { name: '语言' })).toHaveProperty('value', 'zh-CN');
    expect(localStorage.getItem('novelworld.ui.locale')).toBe('zh-CN');
  });

  it('reformats stored local notices while preserving server and user text', async () => {
    const { displayMessage, UiMessageError, setLocale } = await import('./index');
    const error = new UiMessageError({ key: 'Enter this world as {p0}', values: { p0: '云舟 $&' } });
    const serverMessage = '服务端原样消息 {p0} $&';
    expect(displayMessage(error.uiMessage)).toBe('Enter this world as 云舟 $&');
    setLocale('zh-CN');
    expect(displayMessage(error.uiMessage)).toBe('以 云舟 $& 之名，踏入这个世界');
    expect(displayMessage(serverMessage)).toBe(serverMessage);
  });

  it('tolerates storage read and write failures while allowing a live language change', async () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('blocked'); });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('blocked'); });
    const { LanguageSwitcher } = await import('@/shared/ui/LanguageSwitcher');
    const { translate } = await import('./index');
    render(<LanguageSwitcher />);
    expect(translate('Retry')).toBe('Retry');
    fireEvent.change(screen.getByRole('combobox', { name: 'Language' }), { target: { value: 'zh-CN' } });
    expect(translate('Retry')).toBe('重试');
    expect(screen.getByRole('combobox', { name: '语言' })).toHaveProperty('value', 'zh-CN');
  });
});
