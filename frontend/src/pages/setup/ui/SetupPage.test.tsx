import { QueryClientProvider } from '@tanstack/react-query';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { queryClient } from '@/shared/api/queryClient';
import { worldTurnPendingStorageKey } from '@/shared/lib/worldTurnStorage';
import { SetupPage } from './SetupPage';

const mocks = vi.hoisted(() => ({ post: vi.fn() }));

vi.mock('@/shared/api/client', () => ({
  apiClient: { post: mocks.post },
  getApiErrorMessage: () => 'Setup unavailable',
}));

describe('SetupPage', () => {
  beforeEach(() => {
    mocks.post.mockReset();
    queryClient.clear();
    localStorage.clear();
    sessionStorage.clear();
  });

  const renderSetup = (onComplete = vi.fn()) => render(
    <QueryClientProvider client={queryClient}>
      <SetupPage onComplete={onComplete} />
    </QueryClientProvider>,
  );

  it('asks only for the first administrator', () => {
    renderSetup();

    expect(screen.getByRole('heading', { name: '欢迎使用 NovelWorld' })).toBeTruthy();
    expect(screen.getByRole('heading', { name: '创建管理员账户' })).toBeTruthy();
    expect(screen.queryByLabelText('API Key')).toBeNull();
    expect(screen.getByRole('button', { name: '创建管理员并继续' })).toBeTruthy();
  });

  it('creates only the first administrator and stores returned tokens', async () => {
    queryClient.setQueryData(['private'], 'marker');
    sessionStorage.setItem(worldTurnPendingStorageKey('new-admin', 'novel-a'), 'keep');
    sessionStorage.setItem(worldTurnPendingStorageKey('old-user', 'novel-b'), 'remove');
    sessionStorage.setItem('unrelated', 'keep');
    mocks.post.mockResolvedValue({
      data: {
        user: { id: 'new-admin' },
        access_token: 'access',
        refresh_token: 'refresh',
      },
    });
    const onComplete = vi.fn();
    renderSetup(onComplete);

    fireEvent.change(screen.getByLabelText('昵称（可选）'), {
      target: { value: 'Admin' },
    });
    fireEvent.change(screen.getByLabelText('邮箱'), {
      target: { value: 'admin@test.invalid' },
    });
    fireEvent.change(screen.getByLabelText('密码（至少 8 位）'), {
      target: { value: 'password123' },
    });
    fireEvent.click(screen.getByRole('button', { name: '创建管理员并继续' }));

    await waitFor(() => expect(onComplete).toHaveBeenCalledOnce());
    expect(mocks.post).toHaveBeenCalledWith('/setup/init', {
      email: 'admin@test.invalid',
      password: 'password123',
      name: 'Admin',
    });
    expect(localStorage.getItem('auth_token')).toBe('access');
    expect(localStorage.getItem('refresh_token')).toBe('refresh');
    expect(localStorage.getItem('api_key')).toBeNull();
    expect(queryClient.getQueryData(['private'])).toBeUndefined();
    expect(sessionStorage.getItem(worldTurnPendingStorageKey('new-admin', 'novel-a'))).toBe('keep');
    expect(sessionStorage.getItem(worldTurnPendingStorageKey('old-user', 'novel-b'))).toBeNull();
    expect(sessionStorage.getItem('unrelated')).toBe('keep');
  });

  it('renders a stable inline error', async () => {
    queryClient.setQueryData(['private'], 'marker');
    mocks.post.mockRejectedValue(new Error('offline'));
    renderSetup();
    fireEvent.change(screen.getByLabelText('邮箱'), {
      target: { value: 'admin@test.invalid' },
    });
    fireEvent.change(screen.getByLabelText('密码（至少 8 位）'), {
      target: { value: 'password123' },
    });
    fireEvent.click(screen.getByRole('button', { name: '创建管理员并继续' }));

    expect((await screen.findByRole('alert')).textContent).toContain('Setup unavailable');
    expect(mocks.post).toHaveBeenCalledWith('/setup/init', {
      email: 'admin@test.invalid',
      password: 'password123',
      name: undefined,
    });
    expect(queryClient.getQueryData(['private'])).toBe('marker');
  });
});
