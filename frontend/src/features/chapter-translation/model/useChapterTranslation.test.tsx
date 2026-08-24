import React, { type PropsWithChildren } from 'react';
import { renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  chapterTranslationRetryDelay,
  shouldRetryChapterTranslation,
  useChapterTranslation,
} from './useChapterTranslation';

const api = vi.hoisted(() => ({ post: vi.fn() }));

vi.mock('@/shared/api/client', () => ({ apiClient: api }));

const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });

function wrapper({ children }: PropsWithChildren) {
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
}

function httpError(status: number, retryAfter?: string) {
  return {
    isAxiosError: true,
    response: {
      status,
      headers: retryAfter === undefined ? {} : { 'retry-after': retryAfter },
    },
  };
}

describe('chapter translation query', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    queryClient.clear();
  });

  it('calls the authenticated novel translation endpoint only when requested', async () => {
    api.post.mockResolvedValue({ data: { content: '第二章' } });
    const { result, rerender } = renderHook(
      ({ enabled }) => useChapterTranslation('novel', 2, 'Chapter two', enabled),
      { initialProps: { enabled: false }, wrapper },
    );

    expect(api.post).not.toHaveBeenCalled();
    rerender({ enabled: true });

    await waitFor(() => expect(result.current.data?.content).toBe('第二章'));
    expect(api.post).toHaveBeenCalledWith(
      '/novels/novel/chapters/2/translation',
      { content: 'Chapter two' },
      { timeout: 190_000 },
    );
  });

  it.each([409, 429])('keeps retrying HTTP %i responses within the busy budget', (status) => {
    const error = httpError(status, '5');

    expect(shouldRetryChapterTranslation(54, error)).toBe(true);
    expect(shouldRetryChapterTranslation(55, error)).toBe(false);
  });

  it('limits ordinary errors to three exponential-backoff retries', () => {
    const error = httpError(500);

    expect(shouldRetryChapterTranslation(2, error)).toBe(true);
    expect(shouldRetryChapterTranslation(3, error)).toBe(false);
    expect(chapterTranslationRetryDelay(0, error)).toBe(1_000);
    expect(chapterTranslationRetryDelay(1, error)).toBe(2_000);
    expect(chapterTranslationRetryDelay(2, error)).toBe(4_000);
  });

  it('honors Retry-After for busy responses and clamps it to one through five seconds', () => {
    expect(chapterTranslationRetryDelay(0, httpError(409, '3'))).toBe(3_000);
    expect(chapterTranslationRetryDelay(0, httpError(429, '0'))).toBe(1_000);
    expect(chapterTranslationRetryDelay(0, httpError(429, '12'))).toBe(5_000);
    expect(chapterTranslationRetryDelay(0, httpError(409))).toBe(5_000);
  });
});
