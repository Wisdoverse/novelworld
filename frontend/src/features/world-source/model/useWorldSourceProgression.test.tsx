import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { PropsWithChildren } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { OpenWorldView, ReadingProgress } from '@/shared/types';
import { useWorldSourceProgression } from './useWorldSourceProgression';
import { readPendingWorldSource } from '@/shared/lib/worldSourceStorage';

const api = vi.hoisted(() => ({ advance: vi.fn(), progress: vi.fn(), source: vi.fn(), world: vi.fn() }));
vi.mock('@/entities/reading-progress', () => ({
  advanceReadingProgress: api.advance, fetchReadingProgress: api.progress,
  readingProgressKeys: { detail: (id: string) => ['reading-progress', id] },
}));
vi.mock('@/entities/narrative', () => ({
  advanceWorldSource: api.source, fetchOpenWorld: api.world,
  narrativeKeys: { openWorld: (id: string) => ['narrative', id, 'open-world'], worldState: (id: string) => ['narrative', id, 'world-state'] },
}));
const progress = { user_id: 'user', current_chapter: 1, reader_identity_type: 'self' } as ReadingProgress;
const view = { session: { turn_number: 19, entry_context: { unlocked_through_chapter: 1 } } } as OpenWorldView;
const expanded = { ...view, session: { ...view.session, source_context: { ...view.session.entry_context, unlocked_through_chapter: 2 } } };
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
    api.progress.mockRejectedValue(new Error('offline'));
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
    expect(navigate).toHaveBeenCalledWith('/reader/novel/3#latest-world-narrative');
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
  it('does not refill private world caches from an old source response after unmount', async () => {
    const commit = deferred(); api.source.mockReturnValue(commit.promise);
    const hook = renderHook(() => useWorldSourceProgression({ novelId: 'novel', progress, routeChapter: 1, navigate: vi.fn() }), { wrapper });
    act(() => hook.result.current.start(view));
    await waitFor(() => expect(api.source).toHaveBeenCalledOnce());
    hook.unmount(); client.clear();
    await act(async () => commit.resolve({ operation_id: 'operation', previous_source_chapter: 1, source_chapter: 2, view: expanded }));
    expect(api.progress).not.toHaveBeenCalled();
    expect(client.getQueryData(['narrative', 'novel', 'open-world'])).toBeUndefined();
  });
});
