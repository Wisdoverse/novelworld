import React, { type PropsWithChildren } from 'react';
import { renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useChapterTranslation } from './useChapterTranslation';

const api = vi.hoisted(() => ({ post: vi.fn() }));

vi.mock('@/shared/api/client', () => ({ apiClient: api }));

const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });

function wrapper({ children }: PropsWithChildren) {
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
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
    );
  });
});
