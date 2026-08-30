import { describe, it, expect, beforeEach, vi } from 'vitest';
import { narrativeKeys } from '@/entities/narrative';
import { apiClient } from '@/shared/api/client';
import { queryClient } from '@/shared/api/queryClient';
import { worldTurnPendingStorageKey } from '@/shared/lib/worldTurnStorage';
import { useAuthStore } from './useAuthStore';

describe('useAuthStore', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    useAuthStore.setState({ user: null, loading: false, authStatus: 'idle' });
    queryClient.clear();
    localStorage.clear();
    sessionStorage.clear();
  });

  it('initial state has no user', () => {
    const state = useAuthStore.getState();
    expect(state.user).toBeNull();
    expect(state.loading).toBe(false);
    expect(state.authStatus).toBe('idle');
  });

  it('clears private queries before exposing a registered principal', async () => {
    queryClient.setQueryData(['private'], 'marker');
    vi.spyOn(apiClient, 'post').mockResolvedValueOnce({
      data: {
        user: { id: 'new-user', email: 'new@example.com', role: 'user' },
        access_token: 'new-access',
        refresh_token: 'new-refresh',
      },
    });
    const cacheAtExposure: unknown[] = [];
    const unsubscribe = useAuthStore.subscribe(state => {
      if (state.user?.id === 'new-user') {
        cacheAtExposure.push(queryClient.getQueryData(['private']));
      }
    });

    await useAuthStore.getState().register('new@example.com', 'secret');
    unsubscribe();

    expect(cacheAtExposure).toEqual([undefined]);
  });

  it('logout revokes by refresh token and clears the local session immediately', async () => {
    const request = vi.spyOn(apiClient, 'post').mockResolvedValue({ data: undefined });
    localStorage.setItem('auth_token', 'test');
    localStorage.setItem('refresh_token', 'test');
    sessionStorage.setItem(worldTurnPendingStorageKey('user-a', 'novel-a'), 'private-a');
    sessionStorage.setItem(worldTurnPendingStorageKey('user-b', 'novel-b'), 'private-b');
    sessionStorage.setItem('unrelated', 'keep');
    queryClient.setQueryData(['private'], 'marker');
    const logout = useAuthStore.getState().logout();
    expect(request).toHaveBeenCalledWith(
      '/auth/logout',
      { refresh_token: 'test' },
    );
    expect(useAuthStore.getState().user).toBeNull();
    expect(localStorage.getItem('auth_token')).toBeNull();
    expect(sessionStorage.getItem(worldTurnPendingStorageKey('user-a', 'novel-a'))).toBeNull();
    expect(sessionStorage.getItem(worldTurnPendingStorageKey('user-b', 'novel-b'))).toBeNull();
    expect(sessionStorage.getItem('unrelated')).toBe('keep');
    expect(queryClient.getQueryData(['private'])).toBeUndefined();
    await logout;
  });

  it('keeps the session on failed erasure and clears it only after server success', async () => {
    const request = vi.spyOn(apiClient, 'delete');
    localStorage.setItem('auth_token', 'access');
    localStorage.setItem('refresh_token', 'refresh');
    sessionStorage.setItem(worldTurnPendingStorageKey('user', 'novel'), 'recoverable');
    useAuthStore.setState({
      user: { id: 'user', email: 'reader@example.com', role: 'user' },
    });

    queryClient.setQueryData(['private'], 'marker');
    request.mockRejectedValueOnce(new Error('cleanup unavailable'));
    await expect(useAuthStore.getState().deleteAccount()).rejects.toThrow('cleanup unavailable');
    expect(useAuthStore.getState().user?.id).toBe('user');
    expect(localStorage.getItem('auth_token')).toBe('access');
    expect(sessionStorage.getItem(worldTurnPendingStorageKey('user', 'novel'))).toBe('recoverable');
    expect(queryClient.getQueryData(['private'])).toBe('marker');

    request.mockResolvedValueOnce({ data: undefined });
    await expect(useAuthStore.getState().deleteAccount()).resolves.toBe(true);
    expect(request).toHaveBeenLastCalledWith('/auth/me');
    expect(useAuthStore.getState().user).toBeNull();
    expect(localStorage.getItem('auth_token')).toBeNull();
    expect(localStorage.getItem('refresh_token')).toBeNull();
    expect(sessionStorage.getItem(worldTurnPendingStorageKey('user', 'novel'))).toBeNull();
    expect(queryClient.getQueryData(['private'])).toBeUndefined();
    request.mockRestore();
  });

  it('does not let a completed deletion for A clear a newer B session', async () => {
    let resolveDelete!: (value: unknown) => void;
    vi.spyOn(apiClient, 'delete').mockImplementationOnce(
      () => new Promise(resolve => { resolveDelete = resolve; }) as ReturnType<typeof apiClient.delete>,
    );
    vi.spyOn(apiClient, 'post').mockResolvedValue({ data: undefined });
    localStorage.setItem('auth_token', 'access-a');
    localStorage.setItem('refresh_token', 'refresh-a');
    useAuthStore.setState({
      user: { id: 'user-a', email: 'a@example.com', role: 'user' },
    });

    const deletion = useAuthStore.getState().deleteAccount();
    useAuthStore.getState().logout();
    localStorage.setItem('auth_token', 'access-b');
    localStorage.setItem('refresh_token', 'refresh-b');
    useAuthStore.setState({
      user: { id: 'user-b', email: 'b@example.com', role: 'user' },
    });
    sessionStorage.setItem(worldTurnPendingStorageKey('user-b', 'novel-b'), 'pending-b');
    queryClient.setQueryData(['principal-b'], 'private-b');
    resolveDelete({ data: undefined });
    await expect(deletion).resolves.toBe(false);

    expect(useAuthStore.getState().user?.id).toBe('user-b');
    expect(localStorage.getItem('auth_token')).toBe('access-b');
    expect(localStorage.getItem('refresh_token')).toBe('refresh-b');
    expect(sessionStorage.getItem(worldTurnPendingStorageKey('user-b', 'novel-b'))).toBe('pending-b');
    expect(queryClient.getQueryData(['principal-b'])).toBe('private-b');
  });

  it('clears private pending actions after authentication becomes invalid', async () => {
    const request = vi.spyOn(apiClient, 'get').mockRejectedValueOnce({
      isAxiosError: true,
      response: { status: 401 },
    });
    localStorage.setItem('auth_token', 'expired');
    localStorage.setItem('refresh_token', 'expired-refresh');
    sessionStorage.setItem(worldTurnPendingStorageKey('old-user', 'novel'), 'private intent');
    sessionStorage.setItem('unrelated', 'keep');
    queryClient.setQueryData(['private'], 'marker');

    await useAuthStore.getState().fetchMe();

    expect(localStorage.getItem('auth_token')).toBeNull();
    expect(localStorage.getItem('refresh_token')).toBeNull();
    expect(sessionStorage.getItem(worldTurnPendingStorageKey('old-user', 'novel'))).toBeNull();
    expect(sessionStorage.getItem('unrelated')).toBe('keep');
    expect(queryClient.getQueryData(['private'])).toBeUndefined();
    expect(useAuthStore.getState().authStatus).toBe('anonymous');
    request.mockRestore();
  });

  it('ignores a late fetchMe success from principal A after principal B logs in', async () => {
    let resolveFetch!: (value: unknown) => void;
    vi.spyOn(apiClient, 'get').mockImplementationOnce(
      () => new Promise(resolve => { resolveFetch = resolve; }) as ReturnType<typeof apiClient.get>,
    );
    localStorage.setItem('auth_token', 'access-a');
    useAuthStore.setState({
      user: { id: 'user-a', email: 'a@example.com', role: 'user' },
    });

    const fetch = useAuthStore.getState().fetchMe();
    localStorage.setItem('auth_token', 'access-b');
    localStorage.setItem('refresh_token', 'refresh-b');
    useAuthStore.setState({
      user: { id: 'user-b', email: 'b@example.com', role: 'user' },
    });
    sessionStorage.setItem(worldTurnPendingStorageKey('user-b', 'novel-b'), 'pending-b');
    queryClient.setQueryData(['principal-b'], 'private-b');
    resolveFetch({
      data: { id: 'user-a', email: 'a@example.com', role: 'user' },
    });
    await fetch;

    expect(useAuthStore.getState().user?.id).toBe('user-b');
    expect(localStorage.getItem('auth_token')).toBe('access-b');
    expect(localStorage.getItem('refresh_token')).toBe('refresh-b');
    expect(sessionStorage.getItem(worldTurnPendingStorageKey('user-b', 'novel-b'))).toBe('pending-b');
    expect(queryClient.getQueryData(['principal-b'])).toBe('private-b');
  });

  it('ignores a late fetchMe 401 from principal A after principal B logs in', async () => {
    let rejectFetch!: (reason: unknown) => void;
    vi.spyOn(apiClient, 'get').mockImplementationOnce(
      () => new Promise((_, reject) => { rejectFetch = reject; }) as ReturnType<typeof apiClient.get>,
    );
    localStorage.setItem('auth_token', 'access-a');
    useAuthStore.setState({
      user: { id: 'user-a', email: 'a@example.com', role: 'user' },
    });

    const fetch = useAuthStore.getState().fetchMe();
    localStorage.setItem('auth_token', 'access-b');
    localStorage.setItem('refresh_token', 'refresh-b');
    useAuthStore.setState({
      user: { id: 'user-b', email: 'b@example.com', role: 'user' },
    });
    sessionStorage.setItem(worldTurnPendingStorageKey('user-b', 'novel-b'), 'pending-b');
    queryClient.setQueryData(['principal-b'], 'private-b');
    rejectFetch({ isAxiosError: true, response: { status: 401 } });
    await fetch;

    expect(useAuthStore.getState().user?.id).toBe('user-b');
    expect(localStorage.getItem('auth_token')).toBe('access-b');
    expect(localStorage.getItem('refresh_token')).toBe('refresh-b');
    expect(sessionStorage.getItem(worldTurnPendingStorageKey('user-b', 'novel-b'))).toBe('pending-b');
    expect(queryClient.getQueryData(['principal-b'])).toBe('private-b');
  });

  it('preserves recovery state when authentication status is only temporarily unavailable', async () => {
    vi.spyOn(apiClient, 'get').mockRejectedValueOnce({
      isAxiosError: true,
      response: { status: 503 },
    });
    localStorage.setItem('auth_token', 'access');
    localStorage.setItem('refresh_token', 'refresh');
    sessionStorage.setItem(worldTurnPendingStorageKey('user-a', 'novel'), 'recoverable');
    useAuthStore.setState({
      user: { id: 'user-a', email: 'reader@example.com', role: 'user' },
    });

    queryClient.setQueryData(['private'], 'marker');
    await useAuthStore.getState().fetchMe();

    expect(useAuthStore.getState().user?.id).toBe('user-a');
    expect(localStorage.getItem('auth_token')).toBe('access');
    expect(localStorage.getItem('refresh_token')).toBe('refresh');
    expect(sessionStorage.getItem(worldTurnPendingStorageKey('user-a', 'novel'))).toBe('recoverable');
    expect(queryClient.getQueryData(['private'])).toBe('marker');
    expect(useAuthStore.getState().authStatus).toBe('error');
  });

  it('reconciles pending actions to the principal returned by fetchMe', async () => {
    vi.spyOn(apiClient, 'get').mockResolvedValueOnce({
      data: { id: 'user-b', email: 'reader-b@example.com', role: 'user' },
    });
    localStorage.setItem('auth_token', 'shared-token');
    sessionStorage.setItem(worldTurnPendingStorageKey('user-a', 'novel-a'), 'private-a');
    sessionStorage.setItem(worldTurnPendingStorageKey('user-b', 'novel-b'), 'recoverable-b');
    sessionStorage.setItem('unrelated', 'keep');
    queryClient.setQueryData(['private'], 'marker');

    await useAuthStore.getState().fetchMe();

    expect(useAuthStore.getState().user?.id).toBe('user-b');
    expect(sessionStorage.getItem(worldTurnPendingStorageKey('user-a', 'novel-a'))).toBeNull();
    expect(sessionStorage.getItem(worldTurnPendingStorageKey('user-b', 'novel-b'))).toBe('recoverable-b');
    expect(sessionStorage.getItem('unrelated')).toBe('keep');
    expect(queryClient.getQueryData(['private'])).toBeUndefined();
  });

  it('keeps same-user recovery on login and removes another principal\'s keys', async () => {
    vi.spyOn(apiClient, 'post').mockResolvedValueOnce({
      data: {
        user: { id: 'user-a', email: 'reader@example.com', role: 'user' },
        access_token: 'new-access',
        refresh_token: 'new-refresh',
      },
    });
    sessionStorage.setItem(worldTurnPendingStorageKey('user-a', 'novel-a'), 'same-user');
    sessionStorage.setItem(worldTurnPendingStorageKey('user-b', 'novel-b'), 'other-user');
    sessionStorage.setItem('unrelated', 'keep');
    queryClient.setQueryData(['private'], 'marker');

    await useAuthStore.getState().login('reader@example.com', 'secret');

    expect(sessionStorage.getItem(worldTurnPendingStorageKey('user-a', 'novel-a'))).toBe('same-user');
    expect(sessionStorage.getItem(worldTurnPendingStorageKey('user-b', 'novel-b'))).toBeNull();
    expect(sessionStorage.getItem('unrelated')).toBe('keep');
    expect(queryClient.getQueryData(['private'])).toBeUndefined();
  });

  it('removes an A chapter before exposing principal B and forces B to read it', async () => {
    const chapterKey = narrativeKeys.chapter('novel', 2, 'self', 2);
    queryClient.setQueryData(chapterKey, {
      chapter_number: 2,
      content: 'principal A only marker',
      generated: true,
    });
    useAuthStore.setState({
      user: { id: 'user-a', email: 'a@example.com', role: 'user' },
    });
    vi.spyOn(apiClient, 'post').mockResolvedValueOnce({
      data: {
        user: { id: 'user-b', email: 'b@example.com', role: 'user' },
        access_token: 'principal-b-access',
        refresh_token: 'principal-b-refresh',
      },
    });
    const cacheAtExposure: unknown[] = [];
    const unsubscribe = useAuthStore.subscribe(state => {
      if (state.user?.id === 'user-b') {
        cacheAtExposure.push(queryClient.getQueryData(chapterKey));
      }
    });

    await useAuthStore.getState().login('b@example.com', 'secret');
    unsubscribe();

    expect(cacheAtExposure).toEqual([undefined]);
    expect(queryClient.getQueryData(chapterKey)).toBeUndefined();
    const principalBChapter = {
      chapter_number: 2,
      content: 'principal B canon',
      generated: false,
    };
    const get = vi.spyOn(apiClient, 'get').mockResolvedValueOnce({ data: principalBChapter });
    await expect(queryClient.fetchQuery({
      queryKey: chapterKey,
      queryFn: () => apiClient.get('/narrative/novel/chapters/2').then(response => response.data),
      staleTime: Infinity,
    })).resolves.toEqual(principalBChapter);
    expect(get).toHaveBeenCalledOnce();
  });

  it('prevents an old in-flight query from refilling the cache after login', async () => {
    const chapterKey = narrativeKeys.chapter('novel', 2, 'self', 2);
    let resolveOldQuery!: (value: unknown) => void;
    const oldResponse = new Promise(resolve => {
      resolveOldQuery = resolve;
    });
    const oldFetch = queryClient.fetchQuery({
      queryKey: chapterKey,
      queryFn: () => oldResponse,
      staleTime: Infinity,
    }).catch(error => error);
    vi.spyOn(apiClient, 'post').mockResolvedValueOnce({
      data: {
        user: { id: 'user-b', email: 'b@example.com', role: 'user' },
        access_token: 'principal-b-access',
        refresh_token: 'principal-b-refresh',
      },
    });

    await useAuthStore.getState().login('b@example.com', 'secret');
    expect(queryClient.getQueryData(chapterKey)).toBeUndefined();

    resolveOldQuery({
      chapter_number: 2,
      content: 'late principal A marker',
      generated: true,
    });
    await oldFetch;
    await Promise.resolve();

    expect(queryClient.getQueryData(chapterKey)).toBeUndefined();
  });

  it('clears private queries when there is no authenticated token', async () => {
    queryClient.setQueryData(['private'], 'marker');

    await useAuthStore.getState().fetchMe();

    expect(queryClient.getQueryData(['private'])).toBeUndefined();
  });
});
