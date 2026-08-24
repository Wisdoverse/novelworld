import { apiClient } from '@/shared/api/client';

export type LlmUsageScope = 'platform' | 'user';

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
