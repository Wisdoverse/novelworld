import { create } from 'zustand';
import { en, zhCN, type MessageKey } from './messages';

export type Locale = 'en' | 'zh-CN';
export type MessageValues = Record<string, string | number>;
export type UiMessage = string | { key: MessageKey; values?: MessageValues };
export const LOCALE_STORAGE_KEY = 'novelworld.ui.locale';

function readLocale(): Locale {
  try {
    return localStorage.getItem(LOCALE_STORAGE_KEY) === 'zh-CN' ? 'zh-CN' : 'en';
  } catch {
    return 'en';
  }
}

const localeStore = create<{ locale: Locale }>(() => ({ locale: readLocale() }));

export function setLocale(locale: Locale) {
  localeStore.setState({ locale });
  try {
    localStorage.setItem(LOCALE_STORAGE_KEY, locale);
  } catch {
    // The interface remains usable when browser storage is unavailable.
  }
}

export function useLocale() {
  return localeStore(state => state.locale);
}

export function translate(key: MessageKey, values: MessageValues = {}): string {
  const message = (localeStore.getState().locale === 'zh-CN' ? zhCN : en)[key];
  return message.replace(/\{(\w+)\}/g, (token, name: string) => String(values[name] ?? token));
}

export function displayMessage(message?: UiMessage): string | undefined {
  return typeof message === 'string' || message === undefined
    ? message : translate(message.key, message.values);
}

export class UiMessageError extends Error {
  constructor(readonly uiMessage: Exclude<UiMessage, string>) {
    super(translate(uiMessage.key, uiMessage.values));
    this.name = 'UiMessageError';
  }
}

export function useI18n() {
  return { locale: useLocale(), t: translate, setLocale };
}

export type { MessageKey } from './messages';
