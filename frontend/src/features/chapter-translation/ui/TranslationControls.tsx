import { translate as t, useLocale } from '@/shared/lib/i18n';
import { Languages } from 'lucide-react';

interface TranslationControlsProps {
  active: boolean;
  isLoading: boolean;
  isError: boolean;
  unavailableReason?: string;
  onToggle: () => void;
  onRetry: () => void;
}

export function TranslationControls({
  active,
  isLoading,
  isError,
  unavailableReason,
  onToggle,
  onRetry,
}: TranslationControlsProps) {
  useLocale();
  return (
    <div className="mt-5 flex flex-wrap items-center justify-center gap-3 text-sm">
      <button
        type="button"
        aria-pressed={active}
        disabled={isLoading || Boolean(unavailableReason)}
        className="tonal-action px-4 py-2"
        onClick={onToggle}
      >
        <Languages size={15} aria-hidden="true" />
        {unavailableReason
          ? t("Translation is unavailable for this chapter")
          : isLoading
            ? t("Translating…")
            : active
              ? t("Show original")
              : isError
                ? t("Translate again")
                : t("Translate into Chinese")}
      </button>
      {unavailableReason ? (
        <span className="text-[#5f6368]">{unavailableReason}</span>
      ) : null}
      {isLoading ? <span role="status" className="text-[#5f6368]">{t("Translating the chapter…")}</span> : null}
      {isError && !unavailableReason ? (
        <span role="alert" className="text-[#b3261e]">
          {t("Translation failed. Showing the original text.")}
          <button type="button" className="ml-2 underline" onClick={onRetry}>{t("Retry")}</button>
        </span>
      ) : null}
    </div>
  );
}
