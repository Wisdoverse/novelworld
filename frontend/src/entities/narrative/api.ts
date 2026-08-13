import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { apiClient } from '@/shared/api/client';
import type { NarrativeNode, PlayerEntry, WorldState } from '@/shared/types';

export interface NarrativeTransition {
  schema_version: 1;
  prompt_version: string;
  canon_model_version: number;
  canonical_checkpoint_chapter: number;
  rendered_narrative: string;
  events: Array<{
    summary: string;
    actor_character_ids: string[];
    location_id: string | null;
  }>;
  relationship_changes: Array<{ character_id: string; delta: number; reason: string }>;
  location_changes: Array<{ location_id: string; state: string; reason: string }>;
  thread_changes: Array<{
    thread_id: string;
    status: 'open' | 'resolved';
    description: string;
  }>;
}

export interface ChoiceResult {
  chapter_number: number;
  consequence: string;
  transition: NarrativeTransition;
  chapter_content: string;
  world_state: WorldState;
}

export interface EffectiveChapter {
  chapter_number: number;
  content: string;
  generated: boolean;
}

export const narrativeKeys = {
  node: (novelId: string, chapter: number) => ['narrative', novelId, 'node', chapter] as const,
  chapter: (novelId: string, chapter: number) => ['narrative', novelId, 'chapter', chapter] as const,
  worldState: (novelId: string) => ['narrative', novelId, 'world-state'] as const,
  playerEntry: (novelId: string) => ['narrative', novelId, 'player-entry'] as const,
};

export interface CreatePlayerEntityInput {
  name: string;
  background: string;
  capabilities: string[];
  location_id: string;
  inventory: string[];
}

export function usePlayerEntry(novelId: string, enabled: boolean) {
  return useQuery({
    queryKey: narrativeKeys.playerEntry(novelId),
    queryFn: () => apiClient
      .get<PlayerEntry>(`/narrative/${novelId}/player-entry`)
      .then(response => response.data),
    enabled: enabled && !!novelId,
    retry: false,
  });
}

export function useCreatePlayerEntity(novelId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: CreatePlayerEntityInput) => apiClient
      .put<PlayerEntry>(`/narrative/${novelId}/player-entry`, input)
      .then(response => response.data),
    onSuccess: (entry) => {
      queryClient.setQueryData(narrativeKeys.playerEntry(novelId), entry);
      void queryClient.invalidateQueries({ queryKey: narrativeKeys.worldState(novelId) });
    },
  });
}

export function useEffectiveChapter(novelId: string, chapter: number, enabled: boolean) {
  return useQuery({
    queryKey: narrativeKeys.chapter(novelId, chapter),
    queryFn: () => apiClient
      .get<EffectiveChapter>(`/narrative/${novelId}/chapters/${chapter}`, { timeout: 5 * 60_000 })
      .then(response => response.data),
    enabled: enabled && !!novelId && chapter >= 1,
    staleTime: Infinity,
    retry: false,
  });
}

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
      queryClient.setQueryData(narrativeKeys.chapter(novelId, result.chapter_number), {
        chapter_number: result.chapter_number,
        content: result.chapter_content,
        generated: true,
      } satisfies EffectiveChapter);
    },
  });
}
