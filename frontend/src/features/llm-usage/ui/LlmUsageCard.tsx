import { translate as t, useLocale } from '@/shared/lib/i18n';
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
  const language = useLocale();
  const currency = currencyForLanguage(language);
  const title = scope === 'platform' ? t("Platform key usage") : t("My key usage");
  const usage = useQuery({
    queryKey: llmUsageKeys.summary(principalId, scope),
    queryFn: getLlmUsage,
    staleTime: 60_000,
  });

  if (usage.isPending) {
    return (
      <section className="surface-card mt-6 flex items-center justify-center p-10" aria-label={t("Loading {p0}", { p0: title })}>
        <Loader2 className="animate-spin text-[#0b57d0]" />
      </section>
    );
  }

  if (usage.isError) {
    return (
      <section className="surface-card mt-6 p-6 sm:p-8" aria-labelledby="llm-usage-heading">
        <h2 id="llm-usage-heading" className="text-xl font-semibold text-[#1f1f1f]">{title}</h2>
        <p className="mt-2 text-sm text-[#5f6368]">{t("Usage statistics are temporarily unavailable. Services continue to record token counts.")}</p>
        <button type="button" onClick={() => usage.refetch()} className="tonal-action mt-5">
          <RefreshCw size={16} /> {t("Retry")}
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
          <p className="text-sm text-[#5f6368]">{t("Last")} {summary.window_days} {t("days · Reported provider usage")}</p>
        </div>
      </div>

      <dl className="grid gap-3 sm:grid-cols-2">
        <UsageValue label={t("Input tokens")} value={formatTokenCount(summary.tokens.input, language)} />
        <UsageValue label={t("Cached input tokens")} value={formatTokenCount(summary.tokens.cached_input, language)} />
        <UsageValue label={t("Output tokens")} value={formatTokenCount(summary.tokens.output, language)} />
        {estimates === undefined ? (
          <UsageValue label={t("Estimated cost ({p0})", { p0: currency })} value={costMicros !== null
          ? formatCurrencyMicros(costMicros, currency, language)
          : t("Current pricing unverified")} />
        ) : estimates.length === 0 ? (
          <UsageValue label={t("Estimated API token cost")} value={t("Current pricing unverified")} />
        ) : estimates.map((estimate) => (
            <UsageValue
              key={estimate.currency}
              label={t("Estimated API token cost ({p0})", { p0: estimate.currency })}
              value={formatEstimate(estimate.minimum_micros, estimate.maximum_micros, estimate.currency, language)}
            />
        ))}
      </dl>

      {unpriced && (
        <p className="mt-4 text-xs leading-5 text-[#5f6368]">
          {t("Current pricing is unverified for {p0} tokens. Estimates include priced tokens only; subscription usage is not converted using API token rates.", { p0: formatTokenCount(summary.unpriced_tokens, language) })}
        </p>
      )}

      {subscriptions.length > 0 && (
        <div className="mt-5 space-y-3">
          {subscriptions.map((reference) => (
            <section key={`${reference.provider}:${reference.model}`} aria-label={t("{p0} {p1} monthly subscription quotes", { p0: reference.provider, p1: reference.model })}>
              <h3 className="text-sm font-semibold text-[#3c4043]">{t("Monthly subscription quotes ·")} {reference.provider} / {reference.model}</h3>
              {reference.plans.length > 0 ? (
                <ul className="mt-1 flex flex-wrap gap-x-4 gap-y-1 text-sm text-[#5f6368]">
                  {reference.plans.map((plan) => (
                    <li key={`${plan.name}:${plan.currency}`}>
                      {plan.name}：{formatCurrencyMicros(plan.monthly_micros, plan.currency, language)} {t("/ month (quoted, not paid)")}
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="mt-1 text-sm text-[#5f6368]">{t("Monthly pricing is unverified.")}{' '}<a className="underline" href={reference.source_url ?? undefined} target="_blank" rel="noreferrer">{t("View official site")}</a>{' '}{t("Subscription usage is not converted using API token rates.")}</p>
              )}
              {referenceDetails(reference)}
            </section>
          ))}
        </div>
      )}

      {references.some((reference) => reference.billing_kind !== 'subscription') && (
        <details className="mt-4 text-xs leading-5 text-[#5f6368]">
          <summary className="cursor-pointer">{t("Official sources and pricing details")}</summary>
          <ul className="mt-2 space-y-2">
            {references.filter((reference) => reference.billing_kind !== 'subscription').map((reference) => (
              <li key={`${reference.provider}:${reference.model}`}>
                <span className="font-medium">{reference.provider} / {reference.model}</span>
                {reference.verified_on && <span> {t("· Verified:")}{reference.verified_on}</span>}
                {reference.note && <p>{reference.note}</p>}
                {reference.source_url && (
                  <a className="underline" href={reference.source_url} target="_blank" rel="noreferrer">{t("Official source")}</a>
                )}
              </li>
            ))}
          </ul>
        </details>
      )}

      <p className="mt-4 text-xs leading-5 text-[#5f6368]">
        {t("These estimates apply reported billing categories and current pricing to recorded tokens; they are not historical bills. Incomplete tier or peak/off-peak data produces a range. Unreported cache discounts and other charges or discounts are excluded. Consult the provider for actual billing.")}
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
      <summary className="cursor-pointer">{t("Official sources and pricing details")}</summary>
      {reference.verified_on && <p className="mt-1">{t("Verified:")}{reference.verified_on}</p>}
      {reference.note && <p>{reference.note}</p>}
      {reference.source_url && (
        <a className="underline" href={reference.source_url} target="_blank" rel="noreferrer">{t("Official source")}</a>
      )}
    </details>
  );
}

function UsageValue({ label, value }: { label: string; value: string }) {
  useLocale();
  return (
    <div className="rounded-2xl border border-[#dadce0] bg-[#f8fafd] p-4">
      <dt className="text-xs font-medium text-[#5f6368]">{label}</dt>
      <dd className="mt-2 text-xl font-semibold text-[#1f1f1f]">{value}</dd>
    </div>
  );
}
