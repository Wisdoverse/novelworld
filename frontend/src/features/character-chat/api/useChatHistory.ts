import { useQuery } from '@tanstack/react-query';

import { apiClient } from '@/shared/api/client';
import type { ChatMessage } from '@/shared/types';

interface ChatHistoryResponse {
  messages: ChatMessage[];
}

export function useChatHistory(
  characterId: string,
  currentChapter: number,
  enabled: boolean,
) {
  return useQuery({
    queryKey: ['chat-history', characterId, currentChapter],
    queryFn: () => apiClient
      .get<ChatHistoryResponse>(`/chat/${characterId}/history`, {
        params: { limit: 50, offset: 0 },
      })
      .then(response => response.data.messages.slice().reverse()),
    enabled: enabled && !!characterId && currentChapter >= 1,
    retry: false,
    staleTime: Infinity,
  });
}
