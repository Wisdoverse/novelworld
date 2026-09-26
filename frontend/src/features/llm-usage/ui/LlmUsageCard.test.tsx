import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { LlmUsageCard } from './LlmUsageCard';

const getLlmUsage = vi.hoisted(() => vi.fn());

vi.mock('@/entities/llm-usage', () => ({
  getLlmUsage,
  llmUsageKeys: {
    summary: (principalId: string, scope: string) => ['llm-usage', principalId, scope],
  },
}));

afterEach(cleanup);

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

  it('shows original-currency estimates separately and preserves ranges', async () => {
    getLlmUsage.mockResolvedValue({
      contract: 1,
      scope: 'user',
      window_days: 30,
      tokens: { input: '10', cached_input: '0', uncached_input: '10', output: '1', total: '11' },
      costs: { usd_micros: null, cny_micros: null },
      unpriced_tokens: '0',
      estimates: [
        { currency: 'CNY', minimum_micros: '0', maximum_micros: '0' },
        { currency: 'USD', minimum_micros: '100000', maximum_micros: '200000' },
      ],
    });
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(<QueryClientProvider client={queryClient}><LlmUsageCard principalId="reader" scope="user" /></QueryClientProvider>);

    expect(await screen.findByText(/0\.00/)).toBeTruthy();
    expect(screen.getByText('API token 费用估算（CNY）').parentElement?.textContent).toMatch(/0\.00/);
    expect(screen.getByText('API token 费用估算（USD）').parentElement?.textContent).toMatch(/0\.1.*0\.2/);
    expect(screen.queryByText(/汇率/)).toBeNull();
  });

  it('marks unknown prices explicitly instead of showing them as free', async () => {
    getLlmUsage.mockResolvedValue({
      contract: 1,
      scope: 'user',
      window_days: 30,
      tokens: { input: '10', cached_input: '0', uncached_input: '10', output: '0', total: '10' },
      costs: { usd_micros: null, cny_micros: null },
      unpriced_tokens: '10',
      estimates: [],
    });
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(<QueryClientProvider client={queryClient}><LlmUsageCard principalId="reader" scope="user" /></QueryClientProvider>);

    expect(await screen.findByText('未核实当前报价')).toBeTruthy();
    const unpricedMessage = screen.getByText(/10 个 token 未核实当前报价；以上重估仅包含已定价部分/);
    expect(unpricedMessage.textContent).not.toContain('${');
    expect(unpricedMessage.textContent).not.toContain('`');
    expect(screen.queryByText(/免费/)).toBeNull();

  });

  it('formats a legacy zero cost rather than treating it as missing', async () => {
    getLlmUsage.mockResolvedValue({
      contract: 1,
      scope: 'user',
      window_days: 30,
      tokens: { input: '10', cached_input: '0', uncached_input: '10', output: '0', total: '10' },
      costs: { usd_micros: null, cny_micros: '0' },
      unpriced_tokens: '0',
    });
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(<QueryClientProvider client={queryClient}><LlmUsageCard principalId="reader" scope="user" /></QueryClientProvider>);

    expect(await screen.findByText(/¥0\.00|CN¥0\.00/)).toBeTruthy();
  });

  it('shows subscription quotes separately from token cost estimates', async () => {
    getLlmUsage.mockResolvedValue({
      contract: 1,
      scope: 'user',
      window_days: 30,
      tokens: { input: '10', cached_input: '0', uncached_input: '10', output: '0', total: '10' },
      costs: { usd_micros: null, cny_micros: null },
      unpriced_tokens: '10',
      estimates: [],
      pricing_references: [{
        provider: 'Provider',
        model: 'coding-plan',
        source_url: 'https://example.com/pricing',
        verified_on: '2026-09-26',
        note: 'Monthly subscription quota; not token-priced.',
        billing_kind: 'subscription',
        plans: [{ name: 'Plus', currency: 'CNY', monthly_micros: '99000000' }],
      }],
    });
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(<QueryClientProvider client={queryClient}><LlmUsageCard principalId="reader" scope="user" /></QueryClientProvider>);

    expect(await screen.findByText(/Plus：.*99\.00.*月（报价，非已付款）/)).toBeTruthy();
    expect(screen.getByText('API token 费用估算')).toBeTruthy();
    expect(screen.queryByText(/¥0\.00/)).toBeNull();
    expect(screen.getByText(/套餐用量不按 API token 单价折算/)).toBeTruthy();
    expect(screen.getByText(/不代表历史账单/)).toBeTruthy();
  });

  it('explains a plan with no reported token usage without inventing a token cost', async () => {
    getLlmUsage.mockResolvedValue({
      contract: 1,
      scope: 'user',
      window_days: 30,
      tokens: { input: '0', cached_input: '0', uncached_input: '0', output: '0', total: '0' },
      costs: { usd_micros: null, cny_micros: null },
      unpriced_tokens: '0',
      estimates: [],
      pricing_references: [{
        provider: 'Provider', model: 'coding-plan', source_url: 'https://example.com/pricing',
        verified_on: null, note: 'Plan only.', billing_kind: 'subscription', plans: [],
      }],
    });
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(<QueryClientProvider client={queryClient}><LlmUsageCard principalId="reader" scope="user" /></QueryClientProvider>);

    expect(await screen.findByText('未核实当前报价')).toBeTruthy();
    expect(screen.getByText(/套餐月费尚未核实/)).toBeTruthy();
    expect(screen.getByRole('link', { name: '查看官网' }).getAttribute('href')).toBe('https://example.com/pricing');
    expect(screen.getByText(/套餐用量不按 API token 单价折算/)).toBeTruthy();
    expect(screen.queryByText(/¥0\.00|CN¥0\.00/)).toBeNull();
  });
});
