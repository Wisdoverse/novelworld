import { describe, it, expect, beforeEach } from 'vitest';
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
});
