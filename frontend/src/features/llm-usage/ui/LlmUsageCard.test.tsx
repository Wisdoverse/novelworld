import React from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { LlmUsageCard } from './LlmUsageCard';

const getLlmUsage = vi.hoisted(() => vi.fn());

vi.mock('@/entities/llm-usage/api', () => ({ getLlmUsage }));

describe('LlmUsageCard', () => {
  beforeEach(() => {
    document.documentElement.lang = 'zh-CN';
    getLlmUsage.mockReset();
    getLlmUsage.mockResolvedValue({
      contract: 1,
      window_days: 30,
      tokens: {
        input: '3000',
        cached_input: '1000',
        uncached_input: '2000',
        output: '500',
        total: '3500',
      },
      costs: { usd_micros: '450000', cny_micros: '3240000' },
      unpriced_tokens: '0',
    });
  });

  it('shows Chinese users the CNY amount', async () => {
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={queryClient}>
        <LlmUsageCard />
      </QueryClientProvider>,
    );

    expect(await screen.findByText(/CN¥3\.24|¥3\.24/)).toBeTruthy();
    expect(screen.getByText('3,000')).toBeTruthy();
  });
});
