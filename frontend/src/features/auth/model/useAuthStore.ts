import { create } from 'zustand';
import { isAxiosError } from 'axios';
import { apiClient } from '@/shared/api/client';
import { clearPrivateQueryCache } from '@/shared/api/queryClient';
import { clearWorldTurnPendingRequests } from '@/shared/lib/worldTurnStorage';
import type { User } from '@/shared/types';

interface AuthState {
  user: User | null;
  loading: boolean;
  authStatus: 'idle' | 'checking' | 'authenticated' | 'anonymous' | 'error';
  register: (email: string, password: string, name?: string) => Promise<void>;
  login: (email: string, password: string) => Promise<void>;
  logout: () => Promise<void>;
  deleteAccount: () => Promise<boolean>;
  fetchMe: () => Promise<void>;
}

let principalRevision = 0;

function clearLocalSession() {
  clearPrivateQueryCache();
  localStorage.removeItem('auth_token');
  localStorage.removeItem('refresh_token');
  clearWorldTurnPendingRequests();
}

export const useAuthStore = create<AuthState>((set, get) => ({
  user: null,
  loading: false,
  authStatus: 'idle',

  register: async (email, password, name) => {
    const revision = ++principalRevision;
    set({ loading: true });
    try {
      const res = await apiClient.post<{
        user: User;
        access_token: string;
        refresh_token: string;
      }>('/auth/register', { email, password, name });
      if (principalRevision !== revision) return;
      clearPrivateQueryCache();
      clearWorldTurnPendingRequests(res.data.user.id);
      localStorage.setItem('auth_token', res.data.access_token);
      localStorage.setItem('refresh_token', res.data.refresh_token);
      set({ user: res.data.user, loading: false, authStatus: 'authenticated' });
    } catch (e) {
      if (principalRevision === revision) set({ loading: false });
      throw e;
    }
  },

  login: async (email, password) => {
    const revision = ++principalRevision;
    set({ loading: true });
    try {
      const res = await apiClient.post<{
        user: User;
        access_token: string;
        refresh_token: string;
      }>('/auth/login', { email, password });
      if (principalRevision !== revision) return;
      clearPrivateQueryCache();
      clearWorldTurnPendingRequests(res.data.user.id);
      localStorage.setItem('auth_token', res.data.access_token);
      localStorage.setItem('refresh_token', res.data.refresh_token);
      set({ user: res.data.user, loading: false, authStatus: 'authenticated' });
    } catch (e) {
      if (principalRevision === revision) set({ loading: false });
      throw e;
    }
  },

  logout: async () => {
    ++principalRevision;
    const refreshToken = localStorage.getItem('refresh_token');
    const revocation = refreshToken
      ? apiClient.post('/auth/logout', { refresh_token: refreshToken })
      : Promise.resolve();
    // Local logout is immediate. Clearing the token pair also fences any
    // refresh response already in flight from resurrecting this session.
    clearLocalSession();
    set({ user: null, loading: false, authStatus: 'anonymous' });
    await revocation.catch(() => undefined);
  },

  deleteAccount: async () => {
    const revision = principalRevision;
    const userId = get().user?.id;
    await apiClient.delete('/auth/me');
    if (principalRevision !== revision || get().user?.id !== userId) return false;
    ++principalRevision;
    clearLocalSession();
    set({ user: null, loading: false, authStatus: 'anonymous' });
    return true;
  },

  fetchMe: async () => {
    const revision = principalRevision;
    const principalAtStart = get().user?.id ?? null;
    if (!localStorage.getItem('auth_token')) {
      clearLocalSession();
      set({ user: null, authStatus: 'anonymous' });
      return;
    }
    set({ authStatus: 'checking' });
    try {
      const res = await apiClient.get<User>('/auth/me');
      if (
        principalRevision !== revision
        || (get().user?.id ?? null) !== principalAtStart
      ) return;
      clearPrivateQueryCache();
      clearWorldTurnPendingRequests(res.data.id);
      set({ user: res.data, authStatus: 'authenticated' });
    } catch (error) {
      if (
        principalRevision !== revision
        || (get().user?.id ?? null) !== principalAtStart
      ) return;
      const status = isAxiosError(error) ? error.response?.status : undefined;
      if (
        status === 401
        || status === 403
        || (!localStorage.getItem('auth_token') && !localStorage.getItem('refresh_token'))
      ) {
        ++principalRevision;
        clearLocalSession();
        set({ user: null, authStatus: 'anonymous' });
      } else {
        set({ authStatus: 'error' });
      }
    }
  },
}));
