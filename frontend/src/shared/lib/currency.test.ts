import { describe, expect, it } from 'vitest';
import { currencyForLanguage, formatCurrencyMicros } from './currency';

describe('currency display', () => {
  it('uses CNY for every Chinese locale and USD otherwise', () => {
    expect(currencyForLanguage('zh-CN')).toBe('CNY');
    expect(currencyForLanguage('zh-Hant-TW')).toBe('CNY');
    expect(currencyForLanguage('en-US')).toBe('USD');
    expect(currencyForLanguage('fr-FR')).toBe('USD');
  });

  it('formats exact API microunits without relabelling currencies', () => {
    expect(formatCurrencyMicros('3240000', 'CNY', 'zh-CN')).toContain('3.24');
    expect(formatCurrencyMicros('450000', 'USD', 'en-US')).toContain('$0.45');
  });
});
