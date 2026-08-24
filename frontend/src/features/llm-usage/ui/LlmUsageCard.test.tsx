import React from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { LlmUsageCard } from './LlmUsageCard';

const getLlmUsage = vi.hoisted(() => vi.fn());

vi.mock('@/entities/llm-usage', () => ({
  getLlmUsage,
  llmUsageKeys: {
    summary: (principalId: string, scope: string) => ['llm-usage', principalId, scope],
  },
}));

describe('LlmUsageCard', () => {
  beforeEach(() => {
    document.documentElement.lang = 'zh-CN';
    getLlmUsage.mockReset();
    getLlmUsage.mockResolvedValue({
      contract: 1,
      scope: 'user',
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
        <LlmUsageCard principalId="reader" scope="user" />
      </QueryClientProvider>,
    );

    expect(await screen.findByText(/CN¥3\.24|¥3\.24/)).toBeTruthy();
    expect(screen.getByText('3,000')).toBeTruthy();
    expect(screen.getByRole('heading', { name: '我的 Key 消耗' })).toBeTruthy();
  });

  it('isolates cached usage by principal and scope', async () => {
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    getLlmUsage
      .mockResolvedValueOnce({
        contract: 1,
        scope: 'user',
        window_days: 30,
        tokens: {
          input: '1000', cached_input: '0', uncached_input: '1000', output: '0', total: '1000',
        },
        costs: { usd_micros: '100000', cny_micros: '1000000' },
        unpriced_tokens: '0',
      })
      .mockResolvedValueOnce({
        contract: 1,
        scope: 'user',
        window_days: 30,
        tokens: {
          input: '2000', cached_input: '0', uncached_input: '2000', output: '0', total: '2000',
        },
        costs: { usd_micros: '200000', cny_micros: '2000000' },
        unpriced_tokens: '0',
      })
      .mockResolvedValueOnce({
        contract: 1,
        scope: 'platform',
        window_days: 30,
        tokens: {
          input: '3000', cached_input: '0', uncached_input: '3000', output: '0', total: '3000',
        },
        costs: { usd_micros: '300000', cny_micros: '3000000' },
        unpriced_tokens: '0',
      });
    const view = render(
      <QueryClientProvider client={queryClient}>
        <LlmUsageCard principalId="reader-a" scope="user" />
      </QueryClientProvider>,
    );
    expect(await screen.findByText(/CN¥1\.00|¥1\.00/)).toBeTruthy();

    view.rerender(
      <QueryClientProvider client={queryClient}>
        <LlmUsageCard principalId="reader-b" scope="user" />
      </QueryClientProvider>,
    );
    expect(await screen.findByText(/CN¥2\.00|¥2\.00/)).toBeTruthy();

    view.rerender(
      <QueryClientProvider client={queryClient}>
        <LlmUsageCard principalId="reader-b" scope="platform" />
      </QueryClientProvider>,
    );
    expect(await screen.findByText(/CN¥3\.00|¥3\.00/)).toBeTruthy();
    expect(screen.getByRole('heading', { name: '平台 Key 消耗' })).toBeTruthy();
    expect(getLlmUsage).toHaveBeenCalledTimes(3);
  });
});
