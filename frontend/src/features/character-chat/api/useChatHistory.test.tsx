import React, { type PropsWithChildren } from 'react';
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
      () => useChatHistory('character', 2, true),
      { wrapper },
    );

    await waitFor(() => expect(result.current.data?.map(item => item.id)).toEqual(['older', 'newer']));
    expect(api.get).toHaveBeenCalledWith('/chat/character/history', {
      params: { limit: 50, offset: 0 },
    });
  });
});
