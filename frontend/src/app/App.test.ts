import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useAuthStore } from '@/features/auth';
import { useChatStore } from '@/features/character-chat';
import { apiClient } from '@/shared/api/client';
import { queryClient } from '@/shared/api/queryClient';
import {
  AppRoutes,
  handleAuthTokenStorageChange,
  resetPrivateClientStateForPrincipalChange,
} from './App';
import { runtimeConfigKeys } from '@/entities/runtime-config';

// App route tests exercise the auth guard, not ShelfPage's own lazy module
// graph (covered by ShelfPage.test.tsx). Keeping this boundary local avoids
// transform contention making session restoration look permanently pending.
vi.mock('@/pages/shelf', () => ({ default: () => '我的书架' }));

function renderAppRoutes(initialEntries = ['/']) {
  return render(React.createElement(
    QueryClientProvider,
    { client: new QueryClient() },
    React.createElement(
      MemoryRouter,
      { initialEntries },
      React.createElement(AppRoutes),
    ),
  ));
}

beforeEach(() => {
  localStorage.clear();
  useAuthStore.setState({ user: null, loading: false, authStatus: 'idle' });
});

describe('principal-scoped query cache', () => {
  it('clears private chat state before a different principal can use it', () => {
    const cancel = vi.fn();
    useChatStore.setState({
      messages: {
        character: [{
          id: 'message',
          role: 'user',
          content: 'private',
          character_id: 'character',
          created_at: new Date(0).toISOString(),
        }],
      },
      cancelStream: { character: cancel },
    });
    expect(resetPrivateClientStateForPrincipalChange('user-a', 'user-a')).toBe('user-a');
    expect(resetPrivateClientStateForPrincipalChange('user-a', 'user-b')).toBe('user-b');
    expect(cancel).toHaveBeenCalledOnce();
    expect(useChatStore.getState().messages).toEqual({});
  });

  it('clears private cache and chat before reloading on a cross-tab token change', () => {
    const cancel = vi.fn();
    const reload = vi.fn();
    queryClient.setQueryData(['private'], 'principal-a');
    queryClient.setQueryData(runtimeConfigKeys.setup, {
      contract: 4,
      configured: true,
      admin_configured: true,
      llm_configured: false,
    });
    useChatStore.setState({
      messages: {
        character: [{
          id: 'message',
          role: 'user',
          content: 'principal A private chat',
          character_id: 'character',
          created_at: new Date(0).toISOString(),
        }],
      },
      cancelStream: { character: cancel },
    });

    expect(handleAuthTokenStorageChange({
      key: 'auth_token',
      oldValue: 'access-a',
      newValue: 'access-b',
    }, reload)).toBe(true);

    expect(cancel).toHaveBeenCalledOnce();
    expect(queryClient.getQueryData(['private'])).toBeUndefined();
    expect(queryClient.getQueryData(runtimeConfigKeys.setup)).toMatchObject({
      configured: true,
      llm_configured: false,
    });
    expect(useChatStore.getState().messages).toEqual({});
    expect(reload).toHaveBeenCalledOnce();
  });

  it('ignores refresh-token events and unchanged access tokens', () => {
    const reload = vi.fn();
    queryClient.setQueryData(['private'], 'keep');
    useChatStore.setState({
      messages: {
        character: [{
          id: 'message',
          role: 'user',
          content: 'keep',
          character_id: 'character',
          created_at: new Date(0).toISOString(),
        }],
      },
    });

    expect(handleAuthTokenStorageChange({
      key: 'refresh_token',
      oldValue: 'refresh-a',
      newValue: 'refresh-b',
    }, reload)).toBe(false);
    expect(handleAuthTokenStorageChange({
      key: 'auth_token',
      oldValue: 'same',
      newValue: 'same',
    }, reload)).toBe(false);

    expect(queryClient.getQueryData(['private'])).toBe('keep');
    expect(useChatStore.getState().messages.character[0].content).toBe('keep');
    expect(reload).not.toHaveBeenCalled();
    queryClient.clear();
    useChatStore.getState().reset();
  });
});

describe('setup status', () => {
  it('fails closed and offers a retry when server truth is unavailable', async () => {
    const request = vi
      .spyOn(apiClient, 'get')
      .mockRejectedValueOnce(new Error('offline'))
      .mockImplementationOnce(() => new Promise(() => undefined));
    renderAppRoutes();

    expect((await screen.findByRole('alert')).textContent).toContain('无法检查服务配置');
    fireEvent.click(screen.getByRole('button', { name: '重试' }));
    await waitFor(() => expect(request).toHaveBeenCalledTimes(2));
    request.mockRestore();
  });

  it('does not trust the retired setup contract during rollout', async () => {
    const request = vi.spyOn(apiClient, 'get').mockResolvedValueOnce({
      data: { contract: 3, configured: false, llm_configured: false },
    });
    renderAppRoutes();

    expect((await screen.findByRole('alert')).textContent).toContain('无法检查服务配置');
    request.mockRestore();
  });

  it('routes the journey registration destination to registration mode', async () => {
    const request = vi.spyOn(apiClient, 'get').mockResolvedValue({
      data: { contract: 4, configured: true, admin_configured: true, llm_configured: false },
    });
    renderAppRoutes();

    expect(await screen.findByRole('heading', { name: /进入故事.*成为其中的玩家/ })).toBeTruthy();
    fireEvent.click(await screen.findByRole('button', { name: /开始你的旅程/ }));
    expect(await screen.findByRole('heading', { name: '创建账号' })).toBeTruthy();
    request.mockRestore();
  });

  it('restores an authenticated session before guarding a refreshed shelf route', async () => {
    localStorage.setItem('auth_token', 'stored-token');
    useAuthStore.setState({ user: null });
    const request = vi.spyOn(apiClient, 'get').mockImplementation(async (url) => {
      if (url === '/setup/status') {
        return {
          data: {
            contract: 4,
            configured: true,
            admin_configured: true,
            llm_configured: false,
          },
        };
      }
      if (url === '/auth/me') {
        return {
          data: {
            id: 'user-id',
            email: 'reader@example.com',
            role: 'user',
          },
        };
      }
      if (url === '/novels') return { data: [] };
      throw new Error(`Unexpected request: ${url}`);
    });

    render(
      React.createElement(
        QueryClientProvider,
        { client: new QueryClient() },
        React.createElement(
          MemoryRouter,
          { initialEntries: ['/shelf'] },
          React.createElement(AppRoutes),
        ),
      ),
    );

    expect(await screen.findByText('我的书架', {}, { timeout: 10_000 })).toBeTruthy();
    expect(screen.queryByRole('heading', { name: '登录' })).toBeNull();
    localStorage.removeItem('auth_token');
    useAuthStore.setState({ user: null });
    request.mockRestore();
  }, 15_000);

  it('keeps a boot session indeterminate on transient auth failure and lets the user retry', async () => {
    localStorage.setItem('auth_token', 'stored-token');
    localStorage.setItem('refresh_token', 'stored-refresh');
    let meAttempts = 0;
    const request = vi.spyOn(apiClient, 'get').mockImplementation(async (url) => {
      if (url === '/setup/status') {
        return {
          data: {
            contract: 4,
            configured: true,
            admin_configured: true,
            llm_configured: false,
          },
        };
      }
      if (url === '/auth/me') {
        meAttempts++;
        if (meAttempts === 1) {
          throw { isAxiosError: true, response: { status: 503 } };
        }
        return {
          data: { id: 'user-id', email: 'reader@example.com', role: 'user' },
        };
      }
      if (url === '/novels') return { data: [] };
      throw new Error(`Unexpected request: ${url}`);
    });

    renderAppRoutes(['/shelf']);

    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('暂时无法确认登录状态');
    expect(screen.getByRole('button', { name: '退出登录' })).toBeTruthy();
    expect(screen.queryByRole('heading', { name: '登录' })).toBeNull();
    expect(localStorage.getItem('auth_token')).toBe('stored-token');
    expect(localStorage.getItem('refresh_token')).toBe('stored-refresh');

    fireEvent.click(screen.getByRole('button', { name: '重试' }));
    expect(await screen.findByText('我的书架', {}, { timeout: 10_000 })).toBeTruthy();
    expect(meAttempts).toBe(2);
    request.mockRestore();
  }, 15_000);
});
