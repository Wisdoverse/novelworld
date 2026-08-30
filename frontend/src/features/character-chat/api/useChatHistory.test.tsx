import type { PropsWithChildren } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ChatMessage } from '@/shared/types';
import { useChatHistory } from './useChatHistory';

const api = vi.hoisted(() => ({ get: vi.fn() }));

vi.mock('@/shared/api/client', () => ({ apiClient: api }));

const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });

function wrapper({ children }: PropsWithChildren) {
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
}

function message(id: string, createdAt: string): ChatMessage {
  return {
    id,
    role: 'user',
    content: id,
    character_id: 'character',
    chapter_context: 1,
    created_at: createdAt,
  };
}

describe('useChatHistory', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    queryClient.clear();
  });

  it('loads the bounded server page and presents its descending rows chronologically', async () => {
    api.get.mockResolvedValue({
      data: {
        messages: [
          message('newer', '2026-08-23T02:00:00Z'),
          message('older', '2026-08-23T01:00:00Z'),
        ],
      },
    });

    const { result } = renderHook(
      () => useChatHistory('character', 2, 'self', true),
      { wrapper },
    );

    await waitFor(() => expect(result.current.data?.map(item => item.id)).toEqual(['older', 'newer']));
    expect(api.get).toHaveBeenCalledWith('/chat/character/history', {
      params: { limit: 50, offset: 0 },
    });
  });

  it('never reuses history across reader identities and retains the same identity cache', async () => {
    api.get
      .mockResolvedValueOnce({ data: { messages: [message('self-only', '2026-08-23T01:00:00Z')] } })
      .mockResolvedValueOnce({ data: { messages: [message('character-a', '2026-08-23T02:00:00Z')] } })
      .mockResolvedValueOnce({ data: { messages: [message('character-b', '2026-08-23T03:00:00Z')] } });

    const { result, rerender } = renderHook(
      ({ identityScope }) => useChatHistory('character', 2, identityScope, true),
      { initialProps: { identityScope: 'self' }, wrapper },
    );

    await waitFor(() => expect(result.current.data?.[0]?.id).toBe('self-only'));

    rerender({ identityScope: 'character:a' });
    expect(result.current.data).toBeUndefined();
    await waitFor(() => expect(result.current.data?.[0]?.id).toBe('character-a'));

    rerender({ identityScope: 'character:a' });
    expect(result.current.data?.[0]?.id).toBe('character-a');
    expect(api.get).toHaveBeenCalledTimes(2);

    rerender({ identityScope: 'character:b' });
    expect(result.current.data).toBeUndefined();
    await waitFor(() => expect(result.current.data?.[0]?.id).toBe('character-b'));
    expect(api.get).toHaveBeenCalledTimes(3);
  });
});
