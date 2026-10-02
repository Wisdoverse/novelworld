import { UiMessageError, type MessageKey } from '@/shared/lib/i18n';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import axios from 'axios';
import { apiClient, getApiErrorCode } from '@/shared/api/client';
import type {
  WorldSourceAdvanceRequest,
  WorldSourceAdvanceResult,
  NarrativeNode,
  GameRuleTemplate,
  OpenWorldView,
  PlayerEntry,
  WorldAction,
  WorldActionKind,
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

class WorldTurnConfirmationUnknownError extends UiMessageError {
  constructor(key: MessageKey) {
    super({ key });
    this.name = 'WorldTurnConfirmationUnknownError';
  }
}

export function isWorldTurnOutcomeUnknown(error: unknown) {
  if (error instanceof WorldTurnConfirmationUnknownError) return true;
  if (!axios.isAxiosError(error)) return false;
  if (!error.response) return true;
  const code = getApiErrorCode(error);
  return code === 'turn_in_progress'
    || code === 'turn_outcome_unknown'
    || code === 'reading_progress_behind_world'
    || error.response.status >= 500;
}

export const narrativeKeys = {
  node: (novelId: string, chapter: number) => ['narrative', novelId, 'node', chapter] as const,
  chapter: (
    novelId: string,
    chapter: number,
    identityScope: string,
    progressBoundary: number,
  ) => [
    'narrative', novelId, 'chapter', chapter, identityScope, progressBoundary,
  ] as const,
  worldState: (novelId: string) => ['narrative', novelId, 'world-state'] as const,
  playerEntry: (novelId: string, checkpoint?: number) => [
    'narrative', novelId, 'player-entry', checkpoint ?? 'current',
  ] as const,
  openWorld: (novelId: string) => ['narrative', novelId, 'open-world'] as const,
  turnConfirmation: (novelId: string, userId: string, turnId?: string) => [
    'narrative', novelId, 'turn-confirmation', userId, turnId,
  ] as const,
};

export interface WorldTurnConfirmation {
  turn_id: string;
  status: 'in_progress' | 'completed' | 'failed';
  memory_projection_status?: 'pending' | 'saved' | 'skipped';
}

export function useWorldTurnConfirmation(
  novelId: string, userId: string, turnId: string | undefined, enabled: boolean,
) {
  const queryClient = useQueryClient();
  return useQuery({
    queryKey: narrativeKeys.turnConfirmation(novelId, userId, turnId),
    queryFn: async ({ signal }) => {
      const { data: confirmation } = await apiClient.get<WorldTurnConfirmation>(
        `/narrative/${novelId}/world/turns/${turnId}`, { signal, timeout: 5_000 },
      );
      if (confirmation.turn_id !== turnId
        || !['in_progress', 'completed', 'failed'].includes(confirmation.status)) {
        throw new WorldTurnConfirmationUnknownError('The saved action could not be confirmed');
      }
      const terminal = confirmation.status === 'failed'
        || (confirmation.status === 'completed'
          && (confirmation.memory_projection_status === 'saved'
            || confirmation.memory_projection_status === 'skipped'));
      if (!terminal) return { confirmation };
      try {
        const { data: refreshedWorld } = await apiClient.get<OpenWorldView>(
          `/narrative/${novelId}/world`, { signal, timeout: 5_000 },
        );
        signal.throwIfAborted();
        if (refreshedWorld.player.user_id !== userId
          || refreshedWorld.player.novel_id !== novelId) {
          throw new WorldTurnConfirmationUnknownError('The latest world belongs to another reader');
        }
        // A cancelled query or cleared session must never repopulate another
        // principal's world cache with a late confirmation response.
        const key = narrativeKeys.openWorld(novelId);
        const current = queryClient.getQueryData<OpenWorldView | null>(key);
        if (current?.player.user_id !== userId
          || current.player.novel_id !== novelId
          || current.session.turn_number > refreshedWorld.session.turn_number
          || Date.parse(current.world_state.updated_at) > Date.parse(refreshedWorld.world_state.updated_at)) {
          return { confirmation };
        }
        queryClient.setQueryData(key, refreshedWorld);
        return { confirmation, refreshedWorld };
      } catch {
        signal.throwIfAborted();
        // Keep the durable status even if the fresh view fails. In particular,
        // a confirmed failure must never offer generation as confirmation.
        return { confirmation };
      }
    },
    enabled: enabled && !!turnId,
    retry: false,
    staleTime: 0,
    refetchInterval: 10_000,
  });
}

export interface CreatePlayerEntityInput {
  checkpoint_chapter: number;
  name: string;
  background: string;
  capabilities: string[];
  location_id: string | null;
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
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: ['narrative', novelId, 'player-entry'],
          refetchType: 'active',
        }),
        queryClient.invalidateQueries({
          queryKey: narrativeKeys.worldState(novelId),
          refetchType: 'active',
        }),
      ]);
    },
  });
}

export function useEffectiveChapter(
  novelId: string,
  chapter: number,
  identityScope: string,
  progressBoundary: number,
  enabled: boolean,
) {
  return useQuery({
    queryKey: narrativeKeys.chapter(novelId, chapter, identityScope, progressBoundary),
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
      .get<NarrativeNode>(`/narrative/${novelId}/${chapter}`, { timeout: 290_000 })
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
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: narrativeKeys.openWorld(novelId),
          refetchType: 'active',
        }),
        queryClient.invalidateQueries({
          queryKey: narrativeKeys.worldState(novelId),
          refetchType: 'active',
        }),
      ]);
    },
  });
}

export async function suggestWorldAction(novelId: string, intent: string, signal: AbortSignal) {
  const { data } = await apiClient.post<{ kind: WorldActionKind | null }>(
    `/narrative/${novelId}/world/action-suggestion`,
    { intent },
    { signal, timeout: 8_000 },
  );
  return data.kind;
}

export function useSubmitWorldTurn(novelId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ action, idempotencyKey, expectedTurnNumber, expectedSourceChapter }: {
      action: WorldAction;
      idempotencyKey: string;
      expectedTurnNumber: number;
      expectedSourceChapter?: number;
    }) => apiClient
      .post<WorldTurnResult>(`/narrative/${novelId}/world/turns`, {
        ...action,
        expected_turn_number: expectedTurnNumber,
        ...(expectedSourceChapter === undefined ? {} : { expected_source_chapter: expectedSourceChapter }),
      }, {
        headers: { 'Idempotency-Key': idempotencyKey },
        timeout: 120_000,
      })
      .then(response => response.data),
    onSuccess: async result => {
      const openWorldKey = narrativeKeys.openWorld(novelId);
      const projectionIsTerminal = result.memory_projection_status === 'saved'
        || result.memory_projection_status === 'skipped';
      try {
        await queryClient.invalidateQueries(
          { queryKey: openWorldKey, refetchType: 'active' },
          { throwOnError: true },
        );
      } catch {
        // The POST already committed. A rejected confirmation GET must never
        // be reclassified as a terminal rejection that unlocks a new action.
        throw new WorldTurnConfirmationUnknownError("The committed action cannot yet be confirmed from the latest world state");
      }
      await queryClient.invalidateQueries({
        queryKey: narrativeKeys.worldState(novelId),
        refetchType: 'active',
      });
      // A terminal POST is authoritative even after its turn falls outside the
      // bounded journal, but the active view must still advance before the
      // form unlocks and accepts an action based on that view.
      if (projectionIsTerminal) return;
      const view = queryClient.getQueryData<OpenWorldView | null>(openWorldKey);
      const journalEntry = view?.journal.find(entry => entry.turn_id === result.turn_id);
      if (!journalEntry) {
        throw new WorldTurnConfirmationUnknownError("The committed action is not yet present in the latest world state");
      }
      if (journalEntry.memory_projection_status !== 'saved'
        && journalEntry.memory_projection_status !== 'skipped') {
        throw new WorldTurnConfirmationUnknownError("The committed action's memory projection is not yet confirmed");
      }
    },
    onError: async error => {
      if (getApiErrorCode(error) === 'turn_in_progress') {
        // Another key owns the single unresolved authority slot. Refresh so
        // the journal can replace this tab's stale request with that exact turn.
        await queryClient.invalidateQueries({
          queryKey: narrativeKeys.openWorld(novelId),
          refetchType: 'active',
        }).catch(() => undefined);
        return;
      }
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
    onSuccess: async () => {
      await Promise.all([
        queryClient.invalidateQueries({
          queryKey: narrativeKeys.worldState(novelId),
          refetchType: 'active',
        }),
        queryClient.invalidateQueries({
          queryKey: ['narrative', novelId, 'chapter'],
          refetchType: 'active',
        }),
      ]);
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

export async function advanceWorldSource(
  novelId: string, request: WorldSourceAdvanceRequest, idempotencyKey: string,
) {
  return (await apiClient.post<WorldSourceAdvanceResult>(
    `/narrative/${novelId}/world/source`, request,
    { headers: { 'Idempotency-Key': idempotencyKey }, timeout: 30_000 },
  )).data;
}

export async function fetchOpenWorld(novelId: string) {
  return (await apiClient.get<OpenWorldView>(`/narrative/${novelId}/world`)).data;
}
