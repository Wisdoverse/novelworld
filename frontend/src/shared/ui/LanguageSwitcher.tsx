import { useI18n } from '@/shared/lib/i18n';

export function LanguageSwitcher() {
  const { locale, t, setLocale } = useI18n();
  return (
    <label className="flex min-h-11 items-center gap-2 text-sm text-[#3c4043]">
      {t('Language')}
      <select
        className="field-control min-h-11 w-auto"
        value={locale}
        onChange={event => setLocale(event.target.value === 'zh-CN' ? 'zh-CN' : 'en')}
      >
        <option value="en" lang="en">English</option>
        <option value="zh-CN" lang="zh-CN">简体中文</option>
      </select>
    </label>
  );
}
