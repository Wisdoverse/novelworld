import { Languages } from 'lucide-react';

interface TranslationControlsProps {
  active: boolean;
  isLoading: boolean;
  isError: boolean;
  onToggle: () => void;
  onRetry: () => void;
}

export function TranslationControls({
  active,
  isLoading,
  isError,
  onToggle,
  onRetry,
}: TranslationControlsProps) {
  return (
    <div className="mt-5 flex flex-wrap items-center justify-center gap-3 text-sm">
      <button
        type="button"
        aria-pressed={active}
        disabled={isLoading}
        className="tonal-action px-4 py-2"
        onClick={onToggle}
      >
        <Languages size={15} aria-hidden="true" />
        {isLoading ? '翻译中…' : active ? '显示原文' : isError ? '重新翻译' : '翻译成中文'}
      </button>
      {isLoading ? <span role="status" className="text-[#5f6368]">正在翻译正文…</span> : null}
      {isError ? (
        <span role="alert" className="text-[#b3261e]">
          翻译失败，当前显示原文。
          <button type="button" className="ml-2 underline" onClick={onRetry}>重试</button>
        </span>
      ) : null}
    </div>
  );
}
