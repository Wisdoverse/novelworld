import { apiClient } from '@/shared/api/client';

export type LlmUsageScope = 'platform' | 'user';
export type LlmUsageCurrency = 'USD' | 'CNY';

export type LlmUsageEstimate = {
  currency: LlmUsageCurrency;
  minimum_micros: string;
  maximum_micros: string;
};

export type LlmUsagePlanQuote = {
  name: string;
  currency: LlmUsageCurrency;
  monthly_micros: string;
};

export type LlmUsagePricingReference = {
  provider: string;
  model: string;
  source_url: string | null;
  verified_on: string | null;
  note: string;
  billing_kind: 'api' | 'subscription' | 'unavailable' | 'operator';
  plans: LlmUsagePlanQuote[];
};

export type LlmUsageSummary = {
  contract: 1;
  scope: LlmUsageScope;
  window_days: number;
  tokens: {
    input: string;
    cached_input: string;
    uncached_input: string;
    output: string;
    total: string;
  };
  costs: {
    usd_micros: string | null;
    cny_micros: string | null;
  };
  unpriced_tokens: string;
  estimates?: LlmUsageEstimate[];
  pricing_references?: LlmUsagePricingReference[];
};

export const llmUsageKeys = {
  summary: (principalId: string, scope: LlmUsageScope) => (
    ['llm-usage', principalId, scope] as const
  ),
};

export async function getLlmUsage(): Promise<LlmUsageSummary> {
  const response = await apiClient.get<LlmUsageSummary>('/settings/llm/usage');
  return response.data;
}
