import { useI18n } from '@/shared/lib/i18n';
import { ChevronDown, Languages } from 'lucide-react';

export function LanguageSwitcher() {
  const { locale, t, setLocale } = useI18n();
  return (
    <label className="relative inline-flex h-11 w-20 shrink-0 items-center sm:w-24">
      <span className="sr-only">{t('Language')}</span>
      <Languages className="pointer-events-none absolute left-2 hidden h-4 w-4 text-[#5f6368] sm:block" aria-hidden="true" />
      <select
        className="h-11 w-full appearance-none rounded-full border border-transparent bg-transparent pl-2 pr-7 text-xs font-medium text-[#3c4043] outline-none transition-colors hover:border-[#e1e3e8] hover:bg-[#f8fafd] focus-visible:border-[#0b57d0] focus-visible:ring-2 focus-visible:ring-[#a8c7fa] sm:pl-8 sm:pr-8 sm:text-sm"
        value={locale}
        onChange={event => setLocale(event.target.value === 'zh-CN' ? 'zh-CN' : 'en')}
      >
        <option value="en" lang="en">EN</option>
        <option value="zh-CN" lang="zh-CN">中文</option>
      </select>
      <ChevronDown className="pointer-events-none absolute right-2 h-4 w-4 text-[#5f6368]" aria-hidden="true" />
    </label>
  );
}
