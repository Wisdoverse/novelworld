import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SetupPage } from './SetupPage';

const mocks = vi.hoisted(() => ({ post: vi.fn() }));

vi.mock('@/shared/api/client', () => ({
  apiClient: { post: mocks.post },
  getApiErrorMessage: () => 'Setup unavailable',
}));

describe('SetupPage', () => {
  beforeEach(() => {
    mocks.post.mockReset();
    localStorage.clear();
  });

  it('presents setup as a clear two-step guided flow', () => {
    render(<SetupPage onComplete={vi.fn()} llmConfigured={false} />);

    expect(screen.getByRole('heading', { name: '欢迎使用 NovelWorld' })).toBeTruthy();
    expect(screen.getByRole('navigation', { name: '设置进度：第 1 步，共 2 步' })).toBeTruthy();
    expect(screen.getByRole('heading', { name: '连接你的 AI 模型' })).toBeTruthy();
    expect(screen.getByRole('button', { name: /DeepSeek/ }).getAttribute('aria-pressed')).toBe('true');
    expect((screen.getByRole('button', { name: /下一步/ }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('creates only the first administrator and stores returned tokens', async () => {
    mocks.post.mockResolvedValue({
      data: { access_token: 'access', refresh_token: 'refresh' },
    });
    const onComplete = vi.fn();
    render(<SetupPage onComplete={onComplete} llmConfigured={false} />);

    fireEvent.change(screen.getByLabelText('API Key'), {
      target: { value: 'deepseek-secret' },
    });
    fireEvent.click(screen.getByRole('button', { name: /下一步/ }));
    fireEvent.change(screen.getByLabelText('昵称（可选）'), {
      target: { value: 'Admin' },
    });
    fireEvent.change(screen.getByLabelText('邮箱'), {
      target: { value: 'admin@test.invalid' },
    });
    fireEvent.change(screen.getByLabelText('密码（至少 8 位）'), {
      target: { value: 'password123' },
    });
    fireEvent.click(screen.getByRole('button', { name: '完成设置' }));

    await waitFor(() => expect(onComplete).toHaveBeenCalledOnce());
    expect(mocks.post).toHaveBeenCalledWith('/setup/init', {
      email: 'admin@test.invalid',
      password: 'password123',
      name: 'Admin',
      provider: 'deepseek',
      api_key: 'deepseek-secret',
    });
    expect(localStorage.getItem('auth_token')).toBe('access');
    expect(localStorage.getItem('refresh_token')).toBe('refresh');
    expect(localStorage.getItem('api_key')).toBeNull();
  });

  it('renders a stable inline error', async () => {
    mocks.post.mockRejectedValue(new Error('offline'));
    render(<SetupPage onComplete={vi.fn()} llmConfigured={true} />);
    fireEvent.change(screen.getByLabelText('邮箱'), {
      target: { value: 'admin@test.invalid' },
    });
    fireEvent.change(screen.getByLabelText('密码（至少 8 位）'), {
      target: { value: 'password123' },
    });
    fireEvent.click(screen.getByRole('button', { name: '完成设置' }));

    expect((await screen.findByRole('alert')).textContent).toContain('Setup unavailable');
    expect(mocks.post).toHaveBeenCalledWith('/setup/init', {
      email: 'admin@test.invalid',
      password: 'password123',
      name: undefined,
    });
  });
});
