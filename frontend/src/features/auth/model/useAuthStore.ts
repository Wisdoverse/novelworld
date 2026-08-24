import { create } from 'zustand';
import { isAxiosError } from 'axios';
import { apiClient } from '@/shared/api/client';
import { clearPrivateQueryCache } from '@/shared/api/queryClient';
import { clearWorldTurnPendingRequests } from '@/shared/lib/worldTurnStorage';
import type { User } from '@/shared/types';

interface AuthState {
  user: User | null;
  loading: boolean;
  register: (email: string, password: string, name?: string) => Promise<void>;
  login: (email: string, password: string) => Promise<void>;
  logout: () => void;
  deleteAccount: () => Promise<boolean>;
  fetchMe: () => Promise<void>;
}

export const useAuthStore = create<AuthState>((set, get) => ({
  user: null,
  loading: false,

  register: async (email, password, name) => {
    set({ loading: true });
    try {
      const res = await apiClient.post<{
        user: User;
        access_token: string;
        refresh_token: string;
      }>('/auth/register', { email, password, name });
      clearPrivateQueryCache();
      clearWorldTurnPendingRequests(res.data.user.id);
      localStorage.setItem('auth_token', res.data.access_token);
      localStorage.setItem('refresh_token', res.data.refresh_token);
      set({ user: res.data.user, loading: false });
    } catch (e) {
      set({ loading: false });
      throw e;
    }
  },

  login: async (email, password) => {
    set({ loading: true });
    try {
      const res = await apiClient.post<{
        user: User;
        access_token: string;
        refresh_token: string;
      }>('/auth/login', { email, password });
      clearPrivateQueryCache();
      clearWorldTurnPendingRequests(res.data.user.id);
      localStorage.setItem('auth_token', res.data.access_token);
      localStorage.setItem('refresh_token', res.data.refresh_token);
      set({ user: res.data.user, loading: false });
    } catch (e) {
      set({ loading: false });
      throw e;
    }
  },

  logout: () => {
    const accessToken = localStorage.getItem('auth_token');
    const refreshToken = localStorage.getItem('refresh_token');
    if (refreshToken) {
      apiClient.post(
        '/auth/logout',
        { refresh_token: refreshToken },
        accessToken ? { headers: { Authorization: `Bearer ${accessToken}` } } : undefined,
      ).catch(() => {});
    }
    clearPrivateQueryCache();
    localStorage.removeItem('auth_token');
    localStorage.removeItem('refresh_token');
    clearWorldTurnPendingRequests();
    set({ user: null });
  },

  deleteAccount: async () => {
    const token = localStorage.getItem('auth_token');
    const userId = get().user?.id;
    await apiClient.delete('/auth/me');
    if (
      localStorage.getItem('auth_token') !== token
      || get().user?.id !== userId
    ) return false;
    clearPrivateQueryCache();
    localStorage.removeItem('auth_token');
    localStorage.removeItem('refresh_token');
    clearWorldTurnPendingRequests();
    set({ user: null });
    return true;
  },

  fetchMe: async () => {
    const token = localStorage.getItem('auth_token');
    if (!token) {
      clearPrivateQueryCache();
      clearWorldTurnPendingRequests();
      set({ user: null });
      return;
    }
    try {
      const res = await apiClient.get<User>('/auth/me');
      if (localStorage.getItem('auth_token') !== token) return;
      clearPrivateQueryCache();
      clearWorldTurnPendingRequests(res.data.id);
      set({ user: res.data });
    } catch (error) {
      if (localStorage.getItem('auth_token') !== token) return;
      const status = isAxiosError(error) ? error.response?.status : undefined;
      if (status === 401 || status === 403) {
        clearPrivateQueryCache();
        localStorage.removeItem('auth_token');
        localStorage.removeItem('refresh_token');
        clearWorldTurnPendingRequests();
        set({ user: null });
      }
    }
  },
}));
