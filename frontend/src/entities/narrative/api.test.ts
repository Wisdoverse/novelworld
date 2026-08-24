import React, { type PropsWithChildren } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { apiClient } from '@/shared/api/client';
import {
  isNarrativeChoiceConflict,
  isWorldTurnOutcomeUnknown,
  narrativeKeys,
  useGenerateGameRules,
  useCreatePlayerEntity,
  useEffectiveChapter,
  useOpenWorld,
  useStartOpenWorld,
  useSubmitNarrativeChoice,
  useSubmitWorldTurn,
} from './api';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(settle => {
    resolve = settle;
  });
  return { promise, resolve };
}

const axiosError = (status?: number, code?: string) => ({
  isAxiosError: true,
  response: status === undefined ? undefined : {
    status,
    data: { error: { code } },
  },
});

const worldTurnRequest = {
  action: { kind: 'travel' as const, target_id: 'gate', intent: '前往城门' },
  idempotencyKey: '80470e95-87cf-4c50-a05c-f7743c43c079',
  expectedTurnNumber: 1,
};
const worldTurnResult = {
  turn_id: worldTurnRequest.idempotencyKey,
  world_state: {
    user_id: 'user',
    novel_id: 'novel',
    updated_at: '2026-08-13T00:00:00Z',
    state: { choices: [], world_events: [] },
  },
};

const terminalWorldTurnResult = {
  ...worldTurnResult,
  memory_projection_status: 'saved' as const,
};

const queryClient = new QueryClient({
  defaultOptions: {
    mutations: { retry: false },
    queries: { retry: false },
  },
});

function wrapper({ children }: PropsWithChildren) {
  return React.createElement(QueryClientProvider, { client: queryClient }, children);
}

describe('narrative error recovery', () => {
  beforeEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    queryClient.clear();
  });

  it('recognizes the typed choice conflict envelope', () => {
    expect(isNarrativeChoiceConflict(axiosError(409, 'choice_conflict'))).toBe(true);
    expect(isNarrativeChoiceConflict(axiosError(409, 'conflict'))).toBe(false);
  });

  it('keeps the idempotency key only while the world-turn outcome is unknown', () => {
    expect(isWorldTurnOutcomeUnknown(new Error('frontend bug'))).toBe(false);
    expect(isWorldTurnOutcomeUnknown(axiosError())).toBe(true);
    expect(isWorldTurnOutcomeUnknown(axiosError(409, 'turn_in_progress'))).toBe(true);
    expect(isWorldTurnOutcomeUnknown(axiosError(409, 'turn_outcome_unknown'))).toBe(true);
    expect(isWorldTurnOutcomeUnknown(axiosError(409, 'reading_progress_behind_world'))).toBe(true);
    expect(isWorldTurnOutcomeUnknown(axiosError(502, 'llm_error'))).toBe(true);
    expect(isWorldTurnOutcomeUnknown(axiosError(422, 'validation_error'))).toBe(false);
    expect(isWorldTurnOutcomeUnknown(axiosError(409, 'conflict'))).toBe(false);
  });

  it('does not reuse an effective chapter across reader identities', async () => {
    let resolveCharacter!: (value: { data: {
      chapter_number: number;
      content: string;
      generated: boolean;
    } }) => void;
    const characterResponse = new Promise<{ data: {
      chapter_number: number;
      content: string;
      generated: boolean;
    } }>(resolve => {
      resolveCharacter = resolve;
    });
    vi.spyOn(apiClient, 'get').mockReturnValue(characterResponse as never);
    queryClient.setQueryData(
      narrativeKeys.chapter('novel', 2, 'self', 5),
      { chapter_number: 2, content: 'self-only marker', generated: true },
    );
    const { result, rerender } = renderHook(
      ({ identityScope, progressBoundary }: {
        identityScope: string;
        progressBoundary: number;
      }) => useEffectiveChapter(
        'novel',
        2,
        identityScope,
        progressBoundary,
        true,
      ),
      {
        wrapper,
        initialProps: { identityScope: 'self', progressBoundary: 5 },
      },
    );

    expect(result.current.data?.content).toBe('self-only marker');
    rerender({ identityScope: 'character:character-id', progressBoundary: 5 });
    expect(result.current.data).toBeUndefined();

    resolveCharacter({
      data: { chapter_number: 2, content: 'canon chapter', generated: false },
    });
    await waitFor(() => expect(result.current.data?.content).toBe('canon chapter'));
  });

  it('keeps a world-turn mutation pending until the authoritative view refreshes', async () => {
    let finishRefresh!: () => void;
    const refresh = new Promise<void>((resolve) => {
      finishRefresh = resolve;
    });
    const invalidate = vi.spyOn(queryClient, 'invalidateQueries').mockImplementation(async () => {
      await refresh;
      queryClient.setQueryData(narrativeKeys.openWorld('novel'), {
        journal: [{
          turn_id: worldTurnResult.turn_id,
          memory_projection_status: 'saved',
        }],
      });
    });
    vi.spyOn(apiClient, 'post').mockResolvedValue({
      data: worldTurnResult,
    } as never);
    const { result } = renderHook(() => useSubmitWorldTurn('novel'), { wrapper });
    let mutation!: Promise<unknown>;

    act(() => {
      mutation = result.current.mutateAsync(worldTurnRequest);
    });

    await waitFor(() => expect(invalidate).toHaveBeenCalledWith(
      { queryKey: narrativeKeys.openWorld('novel'), refetchType: 'active' },
      { throwOnError: true },
    ));
    expect(result.current.isPending).toBe(true);

    finishRefresh();
    await act(async () => {
      await mutation;
    });
    await waitFor(() => expect(result.current.isPending).toBe(false));
  });

  it('accepts a terminal POST after the active view refreshes even without its journal entry', async () => {
    const invalidate = vi.spyOn(queryClient, 'invalidateQueries').mockResolvedValue();
    queryClient.setQueryData(narrativeKeys.openWorld('novel'), { journal: [] });
    vi.spyOn(apiClient, 'post').mockResolvedValue({ data: terminalWorldTurnResult } as never);
    const { result } = renderHook(() => useSubmitWorldTurn('novel'), { wrapper });

    await act(async () => {
      await expect(result.current.mutateAsync(worldTurnRequest)).resolves.toEqual(
        terminalWorldTurnResult,
      );
    });

    expect(invalidate).toHaveBeenCalledWith(
      { queryKey: narrativeKeys.openWorld('novel'), refetchType: 'active' },
      { throwOnError: true },
    );
    expect(apiClient.post).toHaveBeenCalledWith(
      '/narrative/novel/world/turns',
      {
        kind: 'travel',
        target_id: 'gate',
        intent: '前往城门',
        expected_turn_number: 1,
      },
      expect.objectContaining({
        headers: { 'Idempotency-Key': worldTurnRequest.idempotencyKey },
      }),
    );
  });

  it('keeps a terminal POST pending until the active world view refreshes', async () => {
    let finishRefresh!: () => void;
    const refresh = new Promise<void>((resolve) => {
      finishRefresh = resolve;
    });
    vi.spyOn(queryClient, 'invalidateQueries').mockImplementation(async () => {
      await refresh;
    });
    vi.spyOn(apiClient, 'post').mockResolvedValue({ data: terminalWorldTurnResult } as never);
    const { result } = renderHook(() => useSubmitWorldTurn('novel'), { wrapper });
    let mutation!: Promise<unknown>;

    act(() => {
      mutation = result.current.mutateAsync(worldTurnRequest);
    });
    await waitFor(() => expect(result.current.isPending).toBe(true));

    finishRefresh();
    await act(async () => mutation);
    await waitFor(() => expect(result.current.isPending).toBe(false));
  });

  it('keeps a terminal committed request ambiguous when the active view refresh fails', async () => {
    vi.spyOn(queryClient, 'invalidateQueries').mockRejectedValue(
      axiosError(404, 'confirmation_rejected'),
    );
    vi.spyOn(apiClient, 'post').mockResolvedValue({ data: terminalWorldTurnResult } as never);
    const { result } = renderHook(() => useSubmitWorldTurn('novel'), { wrapper });

    await act(async () => {
      await expect(result.current.mutateAsync(worldTurnRequest))
        .rejects.toThrow('已提交行动尚无法从最新世界状态确认');
    });
  });

  it('rejects a committed response while the refreshed view is still stale', async () => {
    vi.spyOn(queryClient, 'invalidateQueries').mockResolvedValue();
    queryClient.setQueryData(narrativeKeys.openWorld('novel'), { journal: [] });
    vi.spyOn(apiClient, 'post').mockResolvedValue({ data: worldTurnResult } as never);
    const { result } = renderHook(() => useSubmitWorldTurn('novel'), { wrapper });

    let mutationError: unknown;
    await act(async () => {
      mutationError = await result.current.mutateAsync(worldTurnRequest).catch(error => error);
    });
    expect(mutationError).toEqual(expect.objectContaining({
      message: '已提交行动尚未出现在最新世界状态中',
    }));
    expect(isWorldTurnOutcomeUnknown(mutationError)).toBe(true);
  });

  it.each([401, 404])(
    'keeps a committed turn ambiguous when its confirmation GET returns %i',
    async status => {
      const confirmationError = axiosError(status, 'confirmation_rejected');
      vi.spyOn(queryClient, 'invalidateQueries').mockRejectedValue(confirmationError);
      vi.spyOn(apiClient, 'post').mockResolvedValue({ data: worldTurnResult } as never);
      const { result } = renderHook(() => useSubmitWorldTurn('novel'), { wrapper });
      let mutationError: unknown;

      await act(async () => {
        mutationError = await result.current
          .mutateAsync(worldTurnRequest)
          .catch(error => error);
      });

      expect(mutationError).not.toBe(confirmationError);
      expect(mutationError).toEqual(expect.objectContaining({
        message: '已提交行动尚无法从最新世界状态确认',
      }));
      expect(isWorldTurnOutcomeUnknown(mutationError)).toBe(true);
    },
  );

  it('keeps the request ambiguous while its committed memory projection is pending', async () => {
    vi.spyOn(queryClient, 'invalidateQueries').mockResolvedValue();
    queryClient.setQueryData(narrativeKeys.openWorld('novel'), {
      journal: [{
        turn_id: worldTurnResult.turn_id,
        memory_projection_status: 'pending',
      }],
    });
    vi.spyOn(apiClient, 'post').mockResolvedValue({ data: worldTurnResult } as never);
    const { result } = renderHook(() => useSubmitWorldTurn('novel'), { wrapper });

    let mutationError: unknown;
    await act(async () => {
      mutationError = await result.current.mutateAsync(worldTurnRequest).catch(error => error);
    });
    expect(mutationError).toEqual(expect.objectContaining({
      message: '已提交行动的记忆投影尚未确认',
    }));
    expect(isWorldTurnOutcomeUnknown(mutationError)).toBe(true);
  });

  it('refreshes the authoritative journal when another turn already owns the slot', async () => {
    const error = axiosError(409, 'turn_in_progress');
    const invalidate = vi.spyOn(queryClient, 'invalidateQueries').mockResolvedValue();
    vi.spyOn(apiClient, 'post').mockRejectedValue(error);
    const { result } = renderHook(() => useSubmitWorldTurn('novel'), { wrapper });

    await act(async () => {
      await expect(result.current.mutateAsync(worldTurnRequest)).rejects.toBe(error);
    });

    expect(invalidate).toHaveBeenCalledWith({
      queryKey: narrativeKeys.openWorld('novel'),
      refetchType: 'active',
    });
  });

  it('invalidates a stale open-world view after any choice conflict response', async () => {
    const error = axiosError(409, 'conflict');
    vi.spyOn(apiClient, 'post').mockRejectedValue(error);
    queryClient.setQueryData(narrativeKeys.openWorld('novel'), null);

    const { result } = renderHook(
      () => useSubmitNarrativeChoice('novel'),
      { wrapper },
    );

    await act(async () => {
      await expect(result.current.mutateAsync({ nodeId: 'node', choiceIndex: 0 }))
        .rejects.toBe(error);
    });

    await waitFor(() => expect(
      queryClient.getQueryState(narrativeKeys.openWorld('novel'))?.isInvalidated,
    ).toBe(true));
  });

  it('honors Retry-After while polling an in-progress game-rule generation', async () => {
    vi.useFakeTimers();
    const template = {
      novel_id: 'novel',
      canon_model_version: 1,
      schema_version: 1,
      prompt_version: '1.0',
      point_budget: 30,
      minimum_score: 8,
      maximum_score: 15,
      attributes: [],
      action_rules: [],
      source_chapters: [1],
    };
    const inProgress = {
      isAxiosError: true,
      response: {
        status: 409,
        data: { error: { code: 'game_rule_generation_in_progress' } },
        headers: { 'retry-after': '3' },
      },
    };
    const post = vi.spyOn(apiClient, 'post')
      .mockRejectedValueOnce(inProgress)
      .mockResolvedValueOnce({ data: template });
    const { result } = renderHook(
      () => useGenerateGameRules('novel'),
      { wrapper },
    );

    let completion: Promise<unknown>;
    act(() => {
      completion = result.current.mutateAsync();
    });
    await vi.advanceTimersByTimeAsync(2_999);
    expect(post).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);

    await expect(completion!).resolves.toEqual(template);
    expect(post).toHaveBeenCalledTimes(2);
  });

  it('cannot publish a delayed choice response after the principal cache is cleared', async () => {
    const response = deferred<unknown>();
    vi.spyOn(apiClient, 'post').mockReturnValue(response.promise as never);
    const choice = renderHook(() => useSubmitNarrativeChoice('novel'), { wrapper });
    let mutation!: Promise<unknown>;

    act(() => {
      mutation = choice.result.current.mutateAsync({ nodeId: 'node', choiceIndex: 0 });
    });
    await waitFor(() => expect(apiClient.post).toHaveBeenCalledOnce());

    queryClient.clear();
    const currentChapter = {
      chapter_number: 2,
      content: 'principal B canon',
      generated: false,
    };
    vi.spyOn(apiClient, 'get').mockResolvedValue({ data: currentChapter } as never);
    const principalB = renderHook(
      () => useEffectiveChapter('novel', 2, 'character:principal-b', 2, true),
      { wrapper },
    );
    await waitFor(() => expect(principalB.result.current.data).toEqual(currentChapter));

    response.resolve({
      data: {
        chapter_number: 2,
        consequence: 'principal A consequence',
        transition: {},
        chapter_content: 'principal A only marker',
        world_state: { user_id: 'principal-a', state: { choices: [] } },
      },
    });
    await act(async () => mutation);

    await waitFor(() => expect(apiClient.get).toHaveBeenCalledTimes(2));
    expect(principalB.result.current.data).toEqual(currentChapter);
    expect(queryClient.getQueryData(narrativeKeys.worldState('novel'))).toBeUndefined();
    expect(queryClient.getQueriesData({ queryKey: ['narrative', 'novel', 'chapter'] }))
      .not.toContainEqual(expect.arrayContaining([
        expect.anything(),
        expect.objectContaining({ content: 'principal A only marker' }),
      ]));
  });

  it('cannot publish a delayed world turn after the principal cache is cleared', async () => {
    const response = deferred<unknown>();
    vi.spyOn(apiClient, 'post').mockReturnValue(response.promise as never);
    const turn = renderHook(() => useSubmitWorldTurn('novel'), { wrapper });
    let mutation!: Promise<unknown>;

    act(() => {
      mutation = turn.result.current.mutateAsync(worldTurnRequest);
    });
    await waitFor(() => expect(apiClient.post).toHaveBeenCalledOnce());

    queryClient.clear();
    const currentView = {
      session: { turn_number: 8 },
      world_state: { user_id: 'principal-b', state: { choices: [] } },
      journal: [],
    };
    vi.spyOn(apiClient, 'get').mockResolvedValue({ data: currentView } as never);
    const principalB = renderHook(() => useOpenWorld('novel', true), { wrapper });
    await waitFor(() => expect(principalB.result.current.data).toEqual(currentView));

    response.resolve({
      data: {
        ...terminalWorldTurnResult,
        world_state: {
          ...terminalWorldTurnResult.world_state,
          user_id: 'principal-a',
          state: { choices: [], world_events: ['principal A only marker'] },
        },
      },
    });
    await act(async () => mutation);

    await waitFor(() => expect(apiClient.get).toHaveBeenCalledTimes(2));
    expect(principalB.result.current.data).toEqual(currentView);
    expect(queryClient.getQueryData(narrativeKeys.worldState('novel'))).toBeUndefined();
  });

  it('does not repopulate player or open-world caches from old principal responses', async () => {
    const playerResponse = deferred<unknown>();
    const worldResponse = deferred<unknown>();
    vi.spyOn(apiClient, 'put').mockReturnValue(playerResponse.promise as never);
    vi.spyOn(apiClient, 'post').mockReturnValue(worldResponse.promise as never);
    const player = renderHook(() => useCreatePlayerEntity('novel'), { wrapper });
    const world = renderHook(() => useStartOpenWorld('novel'), { wrapper });
    let playerMutation!: Promise<unknown>;
    let worldMutation!: Promise<unknown>;

    act(() => {
      playerMutation = player.result.current.mutateAsync({
        checkpoint_chapter: 2,
        name: 'A',
        background: 'A',
        capabilities: [],
        location_id: 'gate',
        inventory: [],
        rules: {
          mode: 'narrative',
          canon_model_version: null,
          template_schema_version: null,
          template_prompt_version: null,
          attributes: {},
        },
      });
      worldMutation = world.result.current.mutateAsync();
    });
    await waitFor(() => {
      expect(apiClient.put).toHaveBeenCalledOnce();
      expect(apiClient.post).toHaveBeenCalledOnce();
    });

    queryClient.clear();
    const playerKey = narrativeKeys.playerEntry('novel', 2);
    const worldKey = narrativeKeys.openWorld('novel');
    const currentPlayer = { player: { name: 'principal B' } };
    const currentWorld = { world_state: { user_id: 'principal-b' } };
    queryClient.setQueryData(playerKey, currentPlayer);
    queryClient.setQueryData(worldKey, currentWorld);

    playerResponse.resolve({ data: { checkpoint_chapter: 2, player: { name: 'principal A' } } });
    worldResponse.resolve({ data: { world_state: { user_id: 'principal-a' } } });
    await act(async () => Promise.all([playerMutation, worldMutation]));

    expect(queryClient.getQueryData(playerKey)).toEqual(currentPlayer);
    expect(queryClient.getQueryData(worldKey)).toEqual(currentWorld);
    expect(queryClient.getQueryData(narrativeKeys.worldState('novel'))).toBeUndefined();
  });
});
