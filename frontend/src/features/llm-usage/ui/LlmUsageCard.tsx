import { useQuery } from '@tanstack/react-query';
import { BarChart3, Loader2, RefreshCw } from 'lucide-react';
import {
  getLlmUsage,
  llmUsageKeys,
  type LlmUsageScope,
} from '@/entities/llm-usage';
import {
  currencyForLanguage,
  formatCurrencyMicros,
  formatTokenCount,
  type DisplayCurrency,
} from '@/shared/lib/currency';

type UsageData = Awaited<ReturnType<typeof getLlmUsage>>;
type UsagePricingReference = NonNullable<UsageData['pricing_references']>[number];

type LlmUsageCardProps = {
  principalId: string;
  scope: LlmUsageScope;
};

export function LlmUsageCard({ principalId, scope }: LlmUsageCardProps) {
  const language = document.documentElement.lang || navigator.language || 'en-US';
  const currency = currencyForLanguage(language);
  const title = scope === 'platform' ? '平台 Key 消耗' : '我的 Key 消耗';
  const usage = useQuery({
    queryKey: llmUsageKeys.summary(principalId, scope),
    queryFn: getLlmUsage,
    staleTime: 60_000,
  });

  if (usage.isPending) {
    return (
      <section className="surface-card mt-6 flex items-center justify-center p-10" aria-label={`正在加载${title}`}>
        <Loader2 className="animate-spin text-[#0b57d0]" />
      </section>
    );
  }

  if (usage.isError) {
    return (
      <section className="surface-card mt-6 p-6 sm:p-8" aria-labelledby="llm-usage-heading">
        <h2 id="llm-usage-heading" className="text-xl font-semibold text-[#1f1f1f]">{title}</h2>
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
  const estimates = summary.estimates;
  const references = summary.pricing_references ?? [];
  const subscriptions = references.filter((reference) => reference.billing_kind === 'subscription');

  return (
    <section className="surface-card mt-6 p-6 sm:p-8" aria-labelledby="llm-usage-heading">
      <div className="mb-6 flex items-center gap-3">
        <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-[#e6f4ea] text-[#137333]">
          <BarChart3 size={20} />
        </div>
        <div>
          <h2 id="llm-usage-heading" className="text-xl font-semibold text-[#1f1f1f]">{title}</h2>
          <p className="text-sm text-[#5f6368]">近 {summary.window_days} 天 · 按 provider 实际 usage 汇总</p>
        </div>
      </div>

      <dl className="grid gap-3 sm:grid-cols-2">
        <UsageValue label="输入 token" value={formatTokenCount(summary.tokens.input, language)} />
        <UsageValue label="其中缓存输入" value={formatTokenCount(summary.tokens.cached_input, language)} />
        <UsageValue label="输出 token" value={formatTokenCount(summary.tokens.output, language)} />
        {estimates === undefined ? (
          <UsageValue label={`估算成本（${currency}）`} value={costMicros !== null
          ? formatCurrencyMicros(costMicros, currency, language)
          : '未核实当前报价'} />
        ) : estimates.length === 0 ? (
          <UsageValue label="API token 费用估算" value="未核实当前报价" />
        ) : estimates.map((estimate) => (
            <UsageValue
              key={estimate.currency}
              label={`API token 费用估算（${estimate.currency}）`}
              value={formatEstimate(estimate.minimum_micros, estimate.maximum_micros, estimate.currency, language)}
            />
        ))}
      </dl>

      {unpriced && (
        <p className="mt-4 text-xs leading-5 text-[#5f6368]">
          {`${formatTokenCount(summary.unpriced_tokens, language)} 个 token 未核实当前报价；以上重估仅包含已定价部分，套餐用量不按 API token 单价折算。`}
        </p>
      )}

      {subscriptions.length > 0 && (
        <div className="mt-5 space-y-3">
          {subscriptions.map((reference) => (
            <section key={`${reference.provider}:${reference.model}`} aria-label={`${reference.provider} ${reference.model} 套餐月费报价`}>
              <h3 className="text-sm font-semibold text-[#3c4043]">套餐月费报价 · {reference.provider} / {reference.model}</h3>
              {reference.plans.length > 0 ? (
                <ul className="mt-1 flex flex-wrap gap-x-4 gap-y-1 text-sm text-[#5f6368]">
                  {reference.plans.map((plan) => (
                    <li key={`${plan.name}:${plan.currency}`}>
                      {plan.name}：{formatCurrencyMicros(plan.monthly_micros, plan.currency, language)} / 月（报价，非已付款）
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="mt-1 text-sm text-[#5f6368]">套餐月费尚未核实，<a className="underline" href={reference.source_url ?? undefined} target="_blank" rel="noreferrer">查看官网</a>。套餐用量不按 API token 单价折算。</p>
              )}
              {referenceDetails(reference)}
            </section>
          ))}
        </div>
      )}

      {references.some((reference) => reference.billing_kind !== 'subscription') && (
        <details className="mt-4 text-xs leading-5 text-[#5f6368]">
          <summary className="cursor-pointer">官网来源与报价说明</summary>
          <ul className="mt-2 space-y-2">
            {references.filter((reference) => reference.billing_kind !== 'subscription').map((reference) => (
              <li key={`${reference.provider}:${reference.model}`}>
                <span className="font-medium">{reference.provider} / {reference.model}</span>
                {reference.verified_on && <span> · 核验日期：{reference.verified_on}</span>}
                {reference.note && <p>{reference.note}</p>}
                {reference.source_url && (
                  <a className="underline" href={reference.source_url} target="_blank" rel="noreferrer">官方来源</a>
                )}
              </li>
            ))}
          </ul>
        </details>
      )}

      <p className="mt-4 text-xs leading-5 text-[#5f6368]">
        以上仅按已报告的计费类别和当前报价重估已记录 token，不代表历史账单；阶梯/峰谷信息不全时显示区间。未上报的缓存优惠及其他费用/优惠未计入，实际账单以官网为准。
      </p>
    </section>
  );
}

function formatEstimate(minimum: string, maximum: string, currency: DisplayCurrency, language: string) {
  const low = formatCurrencyMicros(minimum, currency, language);
  if (minimum === maximum) return low;
  return `${low} – ${formatCurrencyMicros(maximum, currency, language)}`;
}

function referenceDetails(reference: UsagePricingReference) {
  if (!reference.note && !reference.source_url && !reference.verified_on) return null;
  return (
    <details className="mt-2 text-xs leading-5 text-[#5f6368]">
      <summary className="cursor-pointer">官网来源与报价说明</summary>
      {reference.verified_on && <p className="mt-1">核验日期：{reference.verified_on}</p>}
      {reference.note && <p>{reference.note}</p>}
      {reference.source_url && (
        <a className="underline" href={reference.source_url} target="_blank" rel="noreferrer">官方来源</a>
      )}
    </details>
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
