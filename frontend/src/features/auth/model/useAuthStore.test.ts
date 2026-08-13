import { describe, it, expect, beforeEach, vi } from 'vitest';
import { apiClient } from '@/shared/api/client';
import { useAuthStore } from './useAuthStore';

describe('useAuthStore', () => {
  beforeEach(() => {
    useAuthStore.setState({ user: null, loading: false });
    localStorage.clear();
  });

  it('initial state has no user', () => {
    const state = useAuthStore.getState();
    expect(state.user).toBeNull();
    expect(state.loading).toBe(false);
  });

  it('logout clears user and tokens', () => {
    localStorage.setItem('auth_token', 'test');
    localStorage.setItem('refresh_token', 'test');
    useAuthStore.getState().logout();
    expect(useAuthStore.getState().user).toBeNull();
    expect(localStorage.getItem('auth_token')).toBeNull();
  });

  it('keeps the session on failed erasure and clears it only after server success', async () => {
    const request = vi.spyOn(apiClient, 'delete');
    localStorage.setItem('auth_token', 'access');
    localStorage.setItem('refresh_token', 'refresh');
    useAuthStore.setState({
      user: { id: 'user', email: 'reader@example.com', role: 'user' },
    });

    request.mockRejectedValueOnce(new Error('cleanup unavailable'));
    await expect(useAuthStore.getState().deleteAccount()).rejects.toThrow('cleanup unavailable');
    expect(useAuthStore.getState().user?.id).toBe('user');
    expect(localStorage.getItem('auth_token')).toBe('access');

    request.mockResolvedValueOnce({ data: undefined });
    await useAuthStore.getState().deleteAccount();
    expect(request).toHaveBeenLastCalledWith('/auth/me');
    expect(useAuthStore.getState().user).toBeNull();
    expect(localStorage.getItem('auth_token')).toBeNull();
    expect(localStorage.getItem('refresh_token')).toBeNull();
    request.mockRestore();
  });
});
