import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { useEffect, type PropsWithChildren } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { OpenWorldView, ReadingProgress } from '@/shared/types';
import { useWorldSourceProgression } from './useWorldSourceProgression';
import { readPendingWorldSource } from '@/shared/lib/worldSourceStorage';
import { worldTurnPendingStorageKey } from '@/shared/lib/worldTurnStorage';

const api = vi.hoisted(() => ({ advance: vi.fn(), progress: vi.fn(), source: vi.fn(), world: vi.fn() }));
vi.mock('@/entities/reading-progress', () => ({
  advanceReadingProgress: api.advance, fetchReadingProgress: api.progress,
  readingProgressKeys: { detail: (id: string) => ['reading-progress', id] },
}));
vi.mock('@/entities/narrative', () => ({
  advanceWorldSource: api.source, fetchOpenWorld: api.world,
  narrativeKeys: { openWorld: (id: string) => ['narrative', id, 'open-world'], worldState: (id: string) => ['narrative', id, 'world-state'] },
}));
const progress = { user_id: 'user', novel_id: 'novel', current_chapter: 1, reader_identity_type: 'self' } as ReadingProgress;
const view = { session: { turn_number: 19, entry_context: { unlocked_through_chapter: 1 } } } as OpenWorldView;
const expanded = { ...view, session: { ...view.session, source_context: { ...view.session.entry_context, unlocked_through_chapter: 2 } } };
const terminalTurn = {
  ...view,
  player: { user_id: 'user', novel_id: 'novel' },
  journal: [{ turn_id: 'turn-19', turn_number: 19, expected_source_chapter: 1, memory_projection_status: 'saved' }],
  session: { ...view.session, canonical_events: [] },
} as unknown as OpenWorldView;
let client: QueryClient;
function wrapper({ children }: PropsWithChildren) { return <QueryClientProvider client={client}>{children}</QueryClientProvider>; }
function deferred() {
  let resolve!: (value: unknown) => void;
  return { promise: new Promise(settle => { resolve = settle; }), resolve: (value: unknown) => resolve(value) };
}

describe('world source authority recovery', () => {
  beforeEach(() => {
    vi.resetAllMocks(); window.sessionStorage.clear();
    client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    api.advance.mockResolvedValue({ ...progress, current_chapter: 2 });
    api.source.mockResolvedValue({ operation_id: 'operation', previous_source_chapter: 1, source_chapter: 2, view: expanded });
    api.progress.mockResolvedValue({ ...progress, current_chapter: 2 });
  });
  it('keeps exact identity and committed metadata when authoritative progress is unavailable', async () => {
    api.progress
      .mockResolvedValueOnce(progress)
      .mockRejectedValueOnce(new Error('offline'))
      .mockResolvedValue({ ...progress, current_chapter: 3 });
    const navigate = vi.fn();
    const hook = renderHook(() => useWorldSourceProgression({ novelId: 'novel', progress, routeChapter: 1, navigate }), { wrapper });
    act(() => hook.result.current.start(view));
    await waitFor(() => expect(hook.result.current.isPending).toBe(false));
    expect(hook.result.current.locked).toBe(true);
    expect(navigate).not.toHaveBeenCalled();
    const stored = readPendingWorldSource('user', 'novel');
    expect(stored?.request).toEqual({ expected_turn_number: 19, expected_source_chapter: 1, target_chapter: 2 });
    expect(stored?.result).toEqual({ operation_id: 'operation', previous_source_chapter: 1, source_chapter: 2 });
    api.progress.mockResolvedValue({ ...progress, current_chapter: 3 });
    await act(async () => hook.result.current.recover());
    expect(api.source.mock.calls[1]).toEqual(api.source.mock.calls[0]);
    expect(api.advance.mock.calls[1]).toEqual(['novel', 2, 3]);
    expect(navigate).toHaveBeenCalledWith('/reader/novel/3#latest-world-narrative');
  });
  it('refreshes authoritative turn recovery after busy without discarding the source request', async () => {
    api.source.mockRejectedValue({ isAxiosError: true, response: { status: 409, data: { error: { code: 'world_source_busy' } } } });
    const recovery = { ...view, recoverable_turn: { turn_id: 'original', action: { kind: 'travel', target_id: 'tower', intent: 'continue original' }, expected_turn_number: 19, expected_source_chapter: 1 } };
    api.world.mockResolvedValue(recovery);
    const hook = renderHook(() => useWorldSourceProgression({ novelId: 'novel', progress, routeChapter: 1, navigate: vi.fn() }), { wrapper });
    act(() => hook.result.current.start(view));
    await waitFor(() => expect(hook.result.current.isPending).toBe(false));
    expect(api.world).toHaveBeenCalledOnce();
    expect(client.getQueryData(['narrative', 'novel', 'open-world'])).toEqual(recovery);
    expect(hook.result.current.locked).toBe(true);
    expect(readPendingWorldSource('user', 'novel')?.request).toEqual({ expected_turn_number: 19, expected_source_chapter: 1, target_chapter: 2 });
    expect(readPendingWorldSource('user', 'novel')?.terminal).not.toBe(true);
  });

  it('stops before source admission when the fresh progress snapshot shows an other-tab rewind', async () => {
    api.progress.mockResolvedValue({ ...progress, current_chapter: 1 });
    const cached = { ...progress, current_chapter: 2 };
    const hook = renderHook(() => useWorldSourceProgression({ novelId: 'novel', progress: cached, routeChapter: 2, navigate: vi.fn() }), { wrapper });
    act(() => hook.result.current.advanceIfReady(terminalTurn, 5));
    await waitFor(() => expect(hook.result.current.isPending).toBe(false));
    expect(api.progress).toHaveBeenCalledWith('novel');
    expect(api.advance).not.toHaveBeenCalled();
    expect(api.source).not.toHaveBeenCalled();
    expect(readPendingWorldSource('user', 'novel')?.terminal).toBe(true);
    expect(hook.result.current.error).toContain('自动接续已暂停');
  });

  it('stops before source admission when fresh progress has switched to a character identity', async () => {
    const characterProgress = { ...progress, reader_identity_type: 'character' as const, reader_character_id: 'character' };
    api.progress.mockResolvedValue(characterProgress);
    const navigate = vi.fn();
    const hook = renderHook(({ current }) => useWorldSourceProgression({ novelId: 'novel', progress: current, routeChapter: 1, navigate }), { wrapper, initialProps: { current: progress } });
    act(() => hook.result.current.advanceIfReady(terminalTurn, 5));
    await waitFor(() => expect(hook.result.current.isPending).toBe(false));
    expect(api.advance).not.toHaveBeenCalled();
    expect(api.source).not.toHaveBeenCalled();
    expect(readPendingWorldSource('user', 'novel')?.terminal).toBe(true);
    expect(readPendingWorldSource('user', 'novel')?.notDispatched).toBe(true);
    hook.rerender({ current: characterProgress });
    await waitFor(() => expect(hook.result.current.locked).toBe(false));
    expect(readPendingWorldSource('user', 'novel')).toBeNull();
    expect(navigate).not.toHaveBeenCalled();
  });

  it('keeps the unsent lock until a character rewind route has actually synchronized', async () => {
    const characterProgress = { ...progress, reader_identity_type: 'character' as const, reader_character_id: 'character' };
    api.progress.mockResolvedValue(characterProgress);
    const navigate = vi.fn();
    const hook = renderHook(({ current, route }) => useWorldSourceProgression({ novelId: 'novel', progress: current, routeChapter: route, navigate }), {
      wrapper, initialProps: { current: { ...progress, current_chapter: 3 }, route: 3 },
    });
    act(() => hook.result.current.advanceIfReady(terminalTurn, 5));
    await waitFor(() => expect(hook.result.current.isPending).toBe(false));
    hook.rerender({ current: characterProgress, route: 3 });
    expect(navigate).toHaveBeenCalledWith('/reader/novel/1');
    expect(hook.result.current.locked).toBe(true);
    expect(api.advance).not.toHaveBeenCalled();
    expect(api.source).not.toHaveBeenCalled();
    hook.rerender({ current: characterProgress, route: 1 });
    await waitFor(() => expect(hook.result.current.locked).toBe(false));
    expect(readPendingWorldSource('user', 'novel')).toBeNull();
  });

  it('keeps an unknown older source outcome locked after a character identity switch', async () => {
    api.source.mockRejectedValue(new Error('response lost'));
    const hook = renderHook(({ current }) => useWorldSourceProgression({ novelId: 'novel', progress: current, routeChapter: 1, navigate: vi.fn() }), { wrapper, initialProps: { current: progress } });
    act(() => hook.result.current.start(view));
    await waitFor(() => expect(api.source).toHaveBeenCalledOnce());
    await waitFor(() => expect(hook.result.current.isPending).toBe(false));
    hook.rerender({ current: { ...progress, reader_identity_type: 'character', reader_character_id: 'character' } });
    expect(hook.result.current.locked).toBe(true);
    expect(readPendingWorldSource('user', 'novel')?.notDispatched).not.toBe(true);
  });

  it('does not call source when the guarded owner advance reports a concurrent progress change', async () => {
    api.advance.mockRejectedValue({
      isAxiosError: true,
      response: { status: 409, data: { error: { code: 'reading_progress_changed' } } },
    });
    const hook = renderHook(() => useWorldSourceProgression({ novelId: 'novel', progress, routeChapter: 1, navigate: vi.fn() }), { wrapper });
    act(() => hook.result.current.start(view));
    await waitFor(() => expect(hook.result.current.isPending).toBe(false));
    expect(api.progress).toHaveBeenCalledWith('novel');
    expect(api.advance).toHaveBeenCalledWith('novel', 2, 2);
    expect(api.source).not.toHaveBeenCalled();
    expect(readPendingWorldSource('user', 'novel')?.terminal).not.toBe(true);
  });

  it('does not send the old principal source command after its mount disappears', async () => {
    const unlock = deferred(); api.advance.mockReturnValue(unlock.promise);
    const hook = renderHook(() => useWorldSourceProgression({ novelId: 'novel', progress, routeChapter: 1, navigate: vi.fn() }), { wrapper });
    act(() => hook.result.current.start(view));
    await waitFor(() => expect(api.advance).toHaveBeenCalledOnce());
    hook.unmount();
    await act(async () => unlock.resolve({ ...progress, current_chapter: 2 }));
    expect(api.source).not.toHaveBeenCalled();
    expect(client.getQueryData(['narrative', 'novel', 'open-world'])).toBeUndefined();
  });
  it('wakes a cached book after the previous flight ends and keeps the previous book recoverable', async () => {
    const unlock = deferred();
    api.advance.mockReturnValueOnce(unlock.promise);
    api.progress.mockImplementation(async (id: string) => ({ ...progress, novel_id: id, current_chapter: 2 }));
    api.source.mockImplementation(async (id: string) => ({ operation_id: id, previous_source_chapter: 1, source_chapter: 2,
      view: { ...expanded, player: { ...terminalTurn.player, novel_id: id } },
    }));
    const navigate = vi.fn();
    const hook = renderHook(({ id }) => {
      const controller = useWorldSourceProgression({ novelId: id, progress: { ...progress, novel_id: id }, routeChapter: 1, navigate });
      useEffect(() => controller.advanceIfReady({ ...terminalTurn, player: { ...terminalTurn.player, novel_id: id } }, 5), [id, controller.advanceIfReady]);
      return controller;
    }, { wrapper, initialProps: { id: 'novel' } });
    await waitFor(() => expect(api.advance).toHaveBeenCalledOnce());
    const original = readPendingWorldSource('user', 'novel');
    hook.rerender({ id: 'other' });
    expect(api.advance).toHaveBeenCalledOnce();
    await act(async () => unlock.resolve({ ...progress, current_chapter: 2 }));
    await waitFor(() => expect(api.source).toHaveBeenCalledOnce());
    expect(api.source.mock.calls[0][0]).toBe('other');
    expect(client.getQueryData(['narrative', 'novel', 'open-world'])).toBeUndefined();
    expect(readPendingWorldSource('user', 'novel')).toEqual(original);
    await waitFor(() => expect(hook.result.current.isPending).toBe(false));
    hook.rerender({ id: 'novel' });
    expect(hook.result.current.locked).toBe(true);
    expect(hook.result.current.isPending).toBe(false);
    await act(async () => hook.result.current.recover());
    expect(api.source.mock.calls[1]).toEqual(['novel', original?.request, original?.idempotencyKey]);
  });
  it('preserves the exact older source key when a recovery progress guard rejects', async () => {
    api.source.mockRejectedValueOnce(new Error('response lost'));
    const hook = renderHook(() => useWorldSourceProgression({ novelId: 'novel', progress, routeChapter: 1, navigate: vi.fn() }), { wrapper });
    act(() => hook.result.current.advanceIfReady(terminalTurn, 5));
    await waitFor(() => expect(api.source).toHaveBeenCalledOnce());
    await waitFor(() => expect(hook.result.current.isPending).toBe(false));
    const original = readPendingWorldSource('user', 'novel');
    api.advance.mockRejectedValueOnce({ isAxiosError: true,
      response: { status: 409, data: { error: { code: 'reading_progress_changed' } } },
    });
    await act(async () => hook.result.current.recover());
    expect(readPendingWorldSource('user', 'novel')?.idempotencyKey).toBe(original?.idempotencyKey);
    expect(readPendingWorldSource('user', 'novel')?.terminal).not.toBe(true);
    expect(readPendingWorldSource('user', 'novel')?.notDispatched).not.toBe(true);
    expect(api.source).toHaveBeenCalledOnce();
    await act(async () => hook.result.current.recover());
    expect(api.source.mock.calls[1]).toEqual(api.source.mock.calls[0]);
  });
  it('keeps an older unknown key nonterminal when recovery observes a character identity', async () => {
    api.source.mockRejectedValueOnce(new Error('response lost'));
    const hook = renderHook(() => useWorldSourceProgression({ novelId: 'novel', progress, routeChapter: 1, navigate: vi.fn() }), { wrapper });
    act(() => hook.result.current.advanceIfReady(terminalTurn, 5));
    await waitFor(() => expect(api.source).toHaveBeenCalledOnce());
    await waitFor(() => expect(hook.result.current.isPending).toBe(false));
    const original = readPendingWorldSource('user', 'novel');
    api.progress.mockResolvedValue({ ...progress, reader_identity_type: 'character', reader_character_id: 'character' });
    await act(async () => hook.result.current.recover());
    expect(readPendingWorldSource('user', 'novel')?.idempotencyKey).toBe(original?.idempotencyKey);
    expect(readPendingWorldSource('user', 'novel')?.terminal).not.toBe(true);
    expect(readPendingWorldSource('user', 'novel')?.notDispatched).not.toBe(true);
    expect(api.source).toHaveBeenCalledOnce();
    api.progress.mockResolvedValue({ ...progress, current_chapter: 2 });
    await act(async () => hook.result.current.recover());
    expect(api.source.mock.calls[1]).toEqual(api.source.mock.calls[0]);
  });
  it('does not refill private world caches from an old source response after unmount', async () => {
    const commit = deferred(); api.source.mockReturnValue(commit.promise);
    const hook = renderHook(() => useWorldSourceProgression({ novelId: 'novel', progress, routeChapter: 1, navigate: vi.fn() }), { wrapper });
    act(() => hook.result.current.start(view));
    await waitFor(() => expect(api.source).toHaveBeenCalledOnce());
    hook.unmount(); client.clear();
    await act(async () => commit.resolve({ operation_id: 'operation', previous_source_chapter: 1, source_chapter: 2, view: expanded }));
    expect(api.progress).toHaveBeenCalledOnce(); // pre-admission read only; no post-commit cache refill after unmount.
    expect(client.getQueryData(['narrative', 'novel', 'open-world'])).toBeUndefined();
  });
});

describe('automatic world source progression', () => {
  beforeEach(() => {
    vi.resetAllMocks(); window.sessionStorage.clear();
    client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    api.advance.mockResolvedValue({ ...progress, current_chapter: 2 });
    api.source.mockResolvedValue({ operation_id: 'operation', previous_source_chapter: 1, source_chapter: 2, view: expanded });
    api.progress.mockResolvedValue({ ...progress, current_chapter: 2 });
  });

  it('admits once for the current terminal turn and does not repeat it after expansion or remount', async () => {
    const hook = renderHook(() => useWorldSourceProgression({ novelId: 'novel', progress, routeChapter: 1, navigate: vi.fn() }), { wrapper });
    act(() => hook.result.current.advanceIfReady(terminalTurn, 5));
    await waitFor(() => expect(api.source).toHaveBeenCalledOnce());
    expect(api.source.mock.calls[0]).toEqual(['novel', {
      expected_turn_number: 19, expected_source_chapter: 1, target_chapter: 2,
    }, expect.any(String)]);
    await waitFor(() => expect(hook.result.current.isPending).toBe(false));
    hook.unmount();

    const expandedTurn = { ...terminalTurn, ...expanded, player: terminalTurn.player, journal: terminalTurn.journal } as OpenWorldView;
    const reloaded = renderHook(() => useWorldSourceProgression({ novelId: 'novel', progress: { ...progress, current_chapter: 2 }, routeChapter: 2, navigate: vi.fn() }), { wrapper });
    act(() => reloaded.result.current.advanceIfReady(expandedTurn, 5));
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(api.source).toHaveBeenCalledOnce();
  });

  it('admits the next terminal turn against its own expected source chapter', async () => {
    const navigate = vi.fn();
    const hook = renderHook(() => useWorldSourceProgression({ novelId: 'novel', progress: { ...progress, current_chapter: 2 }, routeChapter: 2, navigate }), { wrapper });
    const next = {
      ...terminalTurn,
      session: { ...terminalTurn.session, turn_number: 20, entry_context: { unlocked_through_chapter: 2 }, source_context: { unlocked_through_chapter: 2 } },
      journal: [{ ...terminalTurn.journal[0], turn_number: 20, expected_source_chapter: 2 }],
    } as OpenWorldView;
    api.advance.mockResolvedValue({ ...progress, current_chapter: 3 });
    api.progress.mockResolvedValue({ ...progress, current_chapter: 3 });
    api.source.mockResolvedValue({ operation_id: 'operation', previous_source_chapter: 2, source_chapter: 3,
      view: { ...next, session: { ...next.session, source_context: { ...next.session.source_context, unlocked_through_chapter: 3 } } } });
    act(() => hook.result.current.advanceIfReady(next, 5));
    await waitFor(() => expect(api.source).toHaveBeenCalledOnce());
    expect(api.source.mock.calls[0][1]).toEqual({ expected_turn_number: 20, expected_source_chapter: 2, target_chapter: 3 });
    await waitFor(() => expect(navigate).toHaveBeenCalledWith('/reader/novel/3#latest-world-narrative'));
  });

  it.each([
    ['scheduled canonical event', (candidate: OpenWorldView) => ({ ...candidate, session: { ...candidate.session, canonical_events: [{ status: 'scheduled' }] } as OpenWorldView['session'] })],
    ['delayed canonical event', (candidate: OpenWorldView) => ({ ...candidate, session: { ...candidate.session, canonical_events: [{ status: 'delayed' }] } as OpenWorldView['session'] })],
    ['pending projection', (candidate: OpenWorldView) => ({ ...candidate, journal: [{ ...candidate.journal[0], memory_projection_status: 'pending' as const }] })],
    ['recoverable turn', (candidate: OpenWorldView) => ({ ...candidate, recoverable_turn: { turn_id: 'pending', action: { kind: 'travel' as const, target_id: 'gate', intent: 'go' }, expected_turn_number: 19 } })],
    ['initial turn without a journal', (candidate: OpenWorldView) => ({ ...candidate, session: { ...candidate.session, turn_number: 0 }, journal: [] })],
    ['end of book', (candidate: OpenWorldView) => candidate],
    ['route and progress mismatch', (candidate: OpenWorldView) => candidate],
    ['reader progress behind source', (candidate: OpenWorldView) => candidate],
    ['old principal', (candidate: OpenWorldView) => ({ ...candidate, player: { ...candidate.player, user_id: 'old-user' } })],
    ['other novel', (candidate: OpenWorldView) => ({ ...candidate, player: { ...candidate.player, novel_id: 'other-novel' } })],
    ['stored pending turn', (candidate: OpenWorldView) => candidate],
  ])('does not admit for %s', async (reason, mutate) => {
    const currentProgress = progress;
    let routeChapter: number | undefined = 1;
    let candidate = terminalTurn;
    let totalChapters = 5;
    if (reason === 'end of book') totalChapters = 1;
    if (reason === 'route and progress mismatch') routeChapter = 2;
    if (reason === 'reader progress behind source') {
      candidate = {
        ...terminalTurn,
        session: { ...terminalTurn.session, entry_context: { unlocked_through_chapter: 2 }, source_context: { unlocked_through_chapter: 2 } },
        journal: [{ ...terminalTurn.journal[0], expected_source_chapter: 2 }],
      } as OpenWorldView;
    }
    if (reason === 'stored pending turn') window.sessionStorage.setItem(worldTurnPendingStorageKey('user', 'novel'), 'pending');
    const hook = renderHook(() => useWorldSourceProgression({ novelId: 'novel', progress: currentProgress, routeChapter, navigate: vi.fn() }), { wrapper });
    act(() => hook.result.current.advanceIfReady(mutate(candidate), totalChapters));
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(api.advance, reason).not.toHaveBeenCalled();
    expect(api.source, reason).not.toHaveBeenCalled();
  });

  it('uses the immutable entry source for legacy turns without chaining after extension', async () => {
    const legacy = {
      ...terminalTurn,
      journal: [{ ...terminalTurn.journal[0], expected_source_chapter: null }],
    } as OpenWorldView;
    const first = renderHook(() => useWorldSourceProgression({ novelId: 'novel', progress, routeChapter: 1, navigate: vi.fn() }), { wrapper });
    act(() => first.result.current.advanceIfReady(legacy, 5));
    await waitFor(() => expect(api.source).toHaveBeenCalledOnce());
    first.unmount();

    const extended = {
      ...legacy,
      session: { ...legacy.session, source_context: { unlocked_through_chapter: 2 } },
    } as OpenWorldView;
    const next = renderHook(() => useWorldSourceProgression({ novelId: 'novel', progress: { ...progress, current_chapter: 2 }, routeChapter: 2, navigate: vi.fn() }), { wrapper });
    act(() => next.result.current.advanceIfReady(extended, 5));
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(api.source).toHaveBeenCalledOnce();
  });
});
