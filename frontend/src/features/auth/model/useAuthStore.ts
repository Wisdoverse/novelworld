import { create } from 'zustand';
import { apiClient } from '@/shared/api/client';
import type { User } from '@/shared/types';

interface AuthState {
  user: User | null;
  loading: boolean;
  register: (email: string, password: string, name?: string) => Promise<void>;
  login: (email: string, password: string) => Promise<void>;
  logout: () => void;
  fetchMe: () => Promise<void>;
}

export const useAuthStore = create<AuthState>((set) => ({
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
      localStorage.setItem('auth_token', res.data.access_token);
      localStorage.setItem('refresh_token', res.data.refresh_token);
      set({ user: res.data.user, loading: false });
    } catch (e) {
      set({ loading: false });
      throw e;
    }
  },

  logout: () => {
    const refreshToken = localStorage.getItem('refresh_token');
    if (refreshToken) {
      apiClient.post('/auth/logout', { refresh_token: refreshToken }).catch(() => {});
    }
    localStorage.removeItem('auth_token');
    localStorage.removeItem('refresh_token');
    set({ user: null });
  },

  fetchMe: async () => {
    const token = localStorage.getItem('auth_token');
    if (!token) return;
    try {
      const res = await apiClient.get<User>('/auth/me');
      set({ user: res.data });
    } catch {
      localStorage.removeItem('auth_token');
      set({ user: null });
    }
  },
}));
