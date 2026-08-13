import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SettingsPage } from './SettingsPage';
import { useAuthStore } from '@/features/auth/model/useAuthStore';

const mocks = vi.hoisted(() => ({ delete: vi.fn(), get: vi.fn(), put: vi.fn() }));

vi.mock('@/shared/api/client', () => ({
  apiClient: { delete: mocks.delete, get: mocks.get, put: mocks.put },
  getApiErrorMessage: () => '设置失败',
}));

describe('SettingsPage', () => {
  beforeEach(() => {
    mocks.get.mockReset();
    mocks.put.mockReset();
    mocks.delete.mockReset();
    useAuthStore.setState({
      user: { id: 'admin', email: 'admin@example.com', role: 'admin' },
      loading: false,
    });
    mocks.get.mockResolvedValue({
      data: {
        provider: 'deepseek',
        model: 'deepseek-v4-flash',
        thinking_enabled: false,
        api_key_configured: true,
      },
    });
  });

  it('updates the DeepSeek model and thinking mode without resending the key', async () => {
    mocks.put.mockResolvedValue({
      data: {
        provider: 'deepseek',
        model: 'deepseek-v4-pro',
        thinking_enabled: true,
        api_key_configured: true,
      },
    });
    render(<MemoryRouter><SettingsPage /></MemoryRouter>);

    await screen.findByRole('heading', { name: '模型设置' });
    fireEvent.change(screen.getByLabelText('模型'), { target: { value: 'deepseek-v4-pro' } });
    fireEvent.click(screen.getByRole('checkbox'));
    fireEvent.click(screen.getByRole('button', { name: '保存模型设置' }));

    await waitFor(() => expect(mocks.put).toHaveBeenCalledWith('/settings/llm', {
      provider: 'deepseek',
      model: 'deepseek-v4-pro',
      thinking_enabled: true,
      api_key: undefined,
    }));
  });

  it('lets every signed-in user explicitly confirm account erasure', async () => {
    useAuthStore.setState({
      user: { id: 'reader', email: 'reader@example.com', role: 'user' },
    });
    localStorage.setItem('auth_token', 'access');
    localStorage.setItem('refresh_token', 'refresh');
    mocks.delete.mockResolvedValue({ data: undefined });
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);

    render(<MemoryRouter><SettingsPage /></MemoryRouter>);
    fireEvent.click(await screen.findByRole('button', { name: '删除账号' }));

    await waitFor(() => expect(mocks.delete).toHaveBeenCalledWith('/auth/me'));
    expect(confirm).toHaveBeenCalledOnce();
    expect(mocks.get).not.toHaveBeenCalled();
    expect(localStorage.getItem('auth_token')).toBeNull();
    confirm.mockRestore();
  });

  it('keeps account erasure available when administrator model settings fail', async () => {
    mocks.get.mockRejectedValueOnce(new Error('model settings unavailable'));

    render(<MemoryRouter><SettingsPage /></MemoryRouter>);

    expect(await screen.findByRole('button', { name: '删除账号' })).toBeTruthy();
  });
});
