import { useQuery } from '@tanstack/react-query';
import { BarChart3, Loader2, RefreshCw } from 'lucide-react';
import { getLlmUsage } from '@/entities/llm-usage/api';
import {
  currencyForLanguage,
  formatCurrencyMicros,
  formatTokenCount,
} from '@/shared/lib/currency';

export function LlmUsageCard() {
  const language = document.documentElement.lang || navigator.language || 'en-US';
  const currency = currencyForLanguage(language);
  const usage = useQuery({
    queryKey: ['llm-usage'],
    queryFn: getLlmUsage,
    staleTime: 60_000,
  });

  if (usage.isPending) {
    return (
      <section className="surface-card mt-6 flex items-center justify-center p-10" aria-label="正在加载 LLM 消耗">
        <Loader2 className="animate-spin text-[#0b57d0]" />
      </section>
    );
  }

  if (usage.isError) {
    return (
      <section className="surface-card mt-6 p-6 sm:p-8" aria-labelledby="llm-usage-heading">
        <h2 id="llm-usage-heading" className="text-xl font-semibold text-[#1f1f1f]">LLM 消耗</h2>
        <p className="mt-2 text-sm text-[#5f6368]">统计服务暂时不可用；token 计数仍由各微服务持续记录。</p>
        <button type="button" onClick={() => usage.refetch()} className="tonal-action mt-5">
          <RefreshCw size={16} /> 重试
        </button>
      </section>
    );
  }

  const summary = usage.data;
  const costMicros = currency === 'CNY' ? summary.costs.cny_micros : summary.costs.usd_micros;
  const unpriced = Number(summary.unpriced_tokens) > 0;

  return (
    <section className="surface-card mt-6 p-6 sm:p-8" aria-labelledby="llm-usage-heading">
      <div className="mb-6 flex items-center gap-3">
        <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-[#e6f4ea] text-[#137333]">
          <BarChart3 size={20} />
        </div>
        <div>
          <h2 id="llm-usage-heading" className="text-xl font-semibold text-[#1f1f1f]">LLM 消耗</h2>
          <p className="text-sm text-[#5f6368]">近 {summary.window_days} 天 · 按 provider 实际 usage 汇总</p>
        </div>
      </div>

      <dl className="grid gap-3 sm:grid-cols-2">
        <UsageValue label="输入 token" value={formatTokenCount(summary.tokens.input, language)} />
        <UsageValue label="其中缓存输入" value={formatTokenCount(summary.tokens.cached_input, language)} />
        <UsageValue label="输出 token" value={formatTokenCount(summary.tokens.output, language)} />
        <UsageValue label={`估算成本（${currency}）`} value={costMicros
          ? formatCurrencyMicros(costMicros, currency, language)
          : '未配置价格或汇率'} />
      </dl>

      {unpriced && (
        <p className="mt-4 text-xs leading-5 text-[#5f6368]">
          {formatTokenCount(summary.unpriced_tokens, language)} 个 token 未配置当前模型价格，金额只包含已定价部分。
        </p>
      )}
    </section>
  );
}

function UsageValue({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-2xl border border-[#dadce0] bg-[#f8fafd] p-4">
      <dt className="text-xs font-medium text-[#5f6368]">{label}</dt>
      <dd className="mt-2 text-xl font-semibold text-[#1f1f1f]">{value}</dd>
    </div>
  );
}
