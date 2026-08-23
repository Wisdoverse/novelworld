import { apiClient } from '@/shared/api/client';

export type LlmUsageSummary = {
  contract: 1;
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

export async function getLlmUsage(): Promise<LlmUsageSummary> {
  const response = await apiClient.get<LlmUsageSummary>('/settings/llm/usage');
  return response.data;
}
