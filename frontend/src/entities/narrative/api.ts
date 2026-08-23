import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import axios from 'axios';
import { apiClient, getApiErrorCode } from '@/shared/api/client';
import type {
  NarrativeNode,
  GameRuleTemplate,
  OpenWorldView,
  PlayerEntry,
  WorldAction,
  WorldState,
  WorldTurnResult,
  PlayerRuleProfile,
} from '@/shared/types';

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

export function isNarrativeChoiceConflict(error: unknown) {
  return getApiErrorCode(error) === 'choice_conflict';
}

export function isWorldTurnOutcomeUnknown(error: unknown) {
  if (!axios.isAxiosError(error) || !error.response) return true;
  const code = getApiErrorCode(error);
  return code === 'turn_in_progress'
    || code === 'turn_outcome_unknown'
    || error.response.status >= 500;
}

export const narrativeKeys = {
  node: (novelId: string, chapter: number) => ['narrative', novelId, 'node', chapter] as const,
  chapter: (novelId: string, chapter: number) => ['narrative', novelId, 'chapter', chapter] as const,
  worldState: (novelId: string) => ['narrative', novelId, 'world-state'] as const,
  playerEntry: (novelId: string, checkpoint?: number) => [
    'narrative', novelId, 'player-entry', checkpoint ?? 'current',
  ] as const,
  openWorld: (novelId: string) => ['narrative', novelId, 'open-world'] as const,
};

export interface CreatePlayerEntityInput {
  checkpoint_chapter: number;
  name: string;
  background: string;
  capabilities: string[];
  location_id: string;
  inventory: string[];
  rules: PlayerRuleProfile;
}

export function useGenerateGameRules(novelId: string) {
  return useMutation({
    mutationFn: () => apiClient
      .post<GameRuleTemplate>(`/narrative/${novelId}/game-rules`, undefined, {
        timeout: 15_000,
      })
      .then(response => response.data),
    retry: (failureCount, error) => failureCount < 180
      && getApiErrorCode(error) === 'game_rule_generation_in_progress',
    retryDelay: (_attempt, error) => {
      const retryAfter = axios.isAxiosError(error)
        ? Number(error.response?.headers['retry-after'])
        : Number.NaN;
      return Number.isFinite(retryAfter) ? retryAfter * 1_000 : 2_000;
    },
  });
}

export function usePlayerEntry(novelId: string, enabled: boolean, checkpoint?: number) {
  return useQuery({
    queryKey: narrativeKeys.playerEntry(novelId, checkpoint),
    queryFn: () => apiClient
      .get<PlayerEntry>(`/narrative/${novelId}/player-entry`, {
        params: checkpoint ? { checkpoint_chapter: checkpoint } : undefined,
      })
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
      queryClient.setQueryData(
        narrativeKeys.playerEntry(novelId, entry.checkpoint_chapter),
        entry,
      );
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

export function useOpenWorld(novelId: string, enabled: boolean) {
  return useQuery({
    queryKey: narrativeKeys.openWorld(novelId),
    queryFn: async () => {
      try {
        return (await apiClient.get<OpenWorldView>(`/narrative/${novelId}/world`)).data;
      } catch (error) {
        if (axios.isAxiosError(error) && error.response?.status === 404) return null;
        throw error;
      }
    },
    enabled: enabled && !!novelId,
    retry: false,
  });
}

export function useStartOpenWorld(novelId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => apiClient
      .post<OpenWorldView>(`/narrative/${novelId}/world`)
      .then(response => response.data),
    onSuccess: view => {
      queryClient.setQueryData(narrativeKeys.openWorld(novelId), view);
      queryClient.setQueryData(narrativeKeys.worldState(novelId), view.world_state);
    },
  });
}

export function useSubmitWorldTurn(novelId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ action, idempotencyKey }: {
      action: WorldAction;
      idempotencyKey: string;
    }) => apiClient
      .post<WorldTurnResult>(`/narrative/${novelId}/world/turns`, action, {
        headers: { 'Idempotency-Key': idempotencyKey },
        timeout: 120_000,
      })
      .then(response => response.data),
    onSuccess: result => {
      queryClient.setQueryData(narrativeKeys.worldState(novelId), result.world_state);
      void queryClient.invalidateQueries({ queryKey: narrativeKeys.openWorld(novelId) });
    },
    onError: async error => {
      if (!isWorldTurnOutcomeUnknown(error)) {
        await Promise.all([
          queryClient.invalidateQueries({ queryKey: narrativeKeys.worldState(novelId) }),
          queryClient.invalidateQueries({ queryKey: narrativeKeys.openWorld(novelId) }),
        ]);
      }
    },
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
    onError: async (error) => {
      if (axios.isAxiosError(error) && error.response?.status === 409) {
        await Promise.all([
          queryClient.invalidateQueries({ queryKey: narrativeKeys.worldState(novelId) }),
          queryClient.invalidateQueries({ queryKey: ['narrative', novelId, 'chapter'] }),
          queryClient.invalidateQueries({ queryKey: narrativeKeys.openWorld(novelId) }),
        ]);
      }
    },
  });
}
