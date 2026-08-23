export type DisplayCurrency = 'CNY' | 'USD';

export function currencyForLanguage(language: string): DisplayCurrency {
  return language.trim().toLowerCase().startsWith('zh') ? 'CNY' : 'USD';
}

export function formatCurrencyMicros(
  micros: string,
  currency: DisplayCurrency,
  language: string,
): string {
  const value = Number(micros);
  if (!Number.isSafeInteger(value) || value < 0) return '—';
  return new Intl.NumberFormat(language || 'en-US', {
    style: 'currency',
    currency,
    minimumFractionDigits: 2,
    maximumFractionDigits: 6,
  }).format(value / 1_000_000);
}

export function formatTokenCount(tokens: string, language: string): string {
  const value = Number(tokens);
  if (!Number.isSafeInteger(value) || value < 0) return '—';
  return new Intl.NumberFormat(language || 'en-US').format(value);
}
