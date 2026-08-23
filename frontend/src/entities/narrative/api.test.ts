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
  useSubmitNarrativeChoice,
} from './api';

const axiosError = (status?: number, code?: string) => ({
  isAxiosError: true,
  response: status === undefined ? undefined : {
    status,
    data: { error: { code } },
  },
});

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
    expect(isWorldTurnOutcomeUnknown(axiosError())).toBe(true);
    expect(isWorldTurnOutcomeUnknown(axiosError(409, 'turn_in_progress'))).toBe(true);
    expect(isWorldTurnOutcomeUnknown(axiosError(409, 'turn_outcome_unknown'))).toBe(true);
    expect(isWorldTurnOutcomeUnknown(axiosError(502, 'llm_error'))).toBe(true);
    expect(isWorldTurnOutcomeUnknown(axiosError(422, 'validation_error'))).toBe(false);
    expect(isWorldTurnOutcomeUnknown(axiosError(409, 'conflict'))).toBe(false);
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
});
