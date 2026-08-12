import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { apiClient } from '@/shared/api/client';
import type { NarrativeNode, WorldState } from '@/shared/types';

export interface ChoiceResult {
  consequence: string;
  world_state: WorldState;
}

export const narrativeKeys = {
  node: (novelId: string, chapter: number) => ['narrative', novelId, 'node', chapter] as const,
  worldState: (novelId: string) => ['narrative', novelId, 'world-state'] as const,
};

export function useNarrativeNode(novelId: string, chapter: number, enabled: boolean) {
  return useQuery({
    queryKey: narrativeKeys.node(novelId, chapter),
    queryFn: () => apiClient
      .get<NarrativeNode>(`/narrative/${novelId}/${chapter}`, { timeout: 90_000 })
      .then(response => response.data),
    enabled: enabled && !!novelId && chapter >= 1,
    staleTime: 5 * 60_000,
    retry: false,
  });
}

export function useWorldState(novelId: string, enabled: boolean) {
  return useQuery({
    queryKey: narrativeKeys.worldState(novelId),
    queryFn: () => apiClient
      .get<WorldState>(`/narrative/${novelId}/world-state`)
      .then(response => response.data),
    enabled: enabled && !!novelId,
  });
}

export function useSubmitNarrativeChoice(novelId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: { nodeId: string; choiceIndex: number }) => apiClient
      .post<ChoiceResult>('/narrative/choose', {
        novel_id: novelId,
        node_id: input.nodeId,
        choice_index: input.choiceIndex,
      }, { timeout: 120_000 })
      .then(response => response.data),
    onSuccess: (result) => {
      queryClient.setQueryData(narrativeKeys.worldState(novelId), result.world_state);
    },
  });
}
