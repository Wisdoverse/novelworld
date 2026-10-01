import type { PropsWithChildren } from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  advanceReadingProgress,
  useReadingProgress,
  useResetReaderIdentity,
  useUpdateReadingProgress,
} from './api';

const api = vi.hoisted(() => ({ get: vi.fn(), put: vi.fn(), post: vi.fn() }));

vi.mock('@/shared/api/client', () => ({ apiClient: api }));

const oldProgress = {
  id: 'progress',
  user_id: 'user',
  novel_id: 'novel',
  current_chapter: 5,
  reader_identity: 'Future',
  reader_identity_type: 'character' as const,
  reader_character_id: 'future-character',
  deviation_mode: 'canon' as const,
  last_read_at: new Date(0).toISOString(),
};

const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });

function wrapper({ children }: PropsWithChildren) {
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
}

describe('reading progress mutations', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    queryClient.clear();
  });

  it('uses monotonic source unlock and preserves an already newer server progress', async () => {
    api.post.mockResolvedValue({ data: oldProgress });
    await expect(advanceReadingProgress('novel', 2)).resolves.toEqual(oldProgress);
    expect(api.post).toHaveBeenCalledWith('/progress/novel/advance', { current_chapter: 2 });
    expect(api.put).not.toHaveBeenCalled();
  });

  it('serializes the optional expected chapter guard', async () => {
    api.post.mockResolvedValue({ data: oldProgress });
    await advanceReadingProgress('novel', 3, 5);
    expect(api.post).toHaveBeenCalledWith('/progress/novel/advance', {
      current_chapter: 3,
      expected_current_chapter: 5,
    });
  });

  it('refetches the complete canonical context after a chapter update', async () => {
    api.get
      .mockResolvedValueOnce({ data: oldProgress })
      .mockResolvedValueOnce({
        data: {
          ...oldProgress,
          current_chapter: 1,
          reader_identity: undefined,
          reader_identity_type: 'self',
          reader_character_id: undefined,
        },
      });
    api.put.mockResolvedValue({});

    const { result } = renderHook(
      () => ({
        progress: useReadingProgress('novel'),
        update: useUpdateReadingProgress('novel'),
      }),
      { wrapper },
    );
    await waitFor(() => expect(result.current.progress.data?.reader_identity).toBe('Future'));

    await act(async () => result.current.update.mutateAsync(1));

    await waitFor(() => expect(result.current.progress.data?.reader_identity_type).toBe('self'));
    expect(result.current.progress.data?.reader_identity).toBeUndefined();
    expect(api.get).toHaveBeenCalledTimes(2);
  });

  it('resets an unavailable character identity and refetches active progress', async () => {
    api.get
      .mockRejectedValueOnce(new Error('reader identity unavailable'))
      .mockResolvedValueOnce({
        data: {
          ...oldProgress,
          reader_identity: undefined,
          reader_identity_type: 'self',
          reader_character_id: undefined,
        },
      });
    api.put.mockResolvedValue({});

    const { result } = renderHook(
      () => ({
        progress: useReadingProgress('novel'),
        resetIdentity: useResetReaderIdentity('novel'),
      }),
      { wrapper },
    );
    await waitFor(() => expect(result.current.progress.isError).toBe(true));

    await act(async () => result.current.resetIdentity.mutateAsync());

    expect(api.put).toHaveBeenCalledWith('/progress/novel/identity', {
      identity_type: 'self',
      identity_name: null,
      character_id: null,
    });
    await waitFor(() => expect(result.current.progress.data?.reader_identity_type).toBe('self'));
    expect(result.current.progress.data?.reader_character_id).toBeUndefined();
    expect(api.get).toHaveBeenCalledTimes(2);
  });
});
