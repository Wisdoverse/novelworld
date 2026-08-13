import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SettingsPage } from './SettingsPage';
import { useAuthStore } from '@/features/auth/model/useAuthStore';

const mocks = vi.hoisted(() => ({
  delete: vi.fn(),
  error: vi.fn(),
  get: vi.fn(),
  put: vi.fn(),
  success: vi.fn(),
}));

vi.mock('sonner', () => ({ toast: { error: mocks.error, success: mocks.success } }));

vi.mock('@/shared/api/client', () => ({
  apiClient: { delete: mocks.delete, get: mocks.get, put: mocks.put },
  getApiErrorMessage: (_error: unknown, fallback: string) => fallback,
}));

describe('SettingsPage', () => {
  beforeEach(() => {
    mocks.get.mockReset();
    mocks.put.mockReset();
    mocks.delete.mockReset();
    mocks.error.mockReset();
    mocks.success.mockReset();
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
    Object.defineProperty(URL, 'createObjectURL', {
      configurable: true,
      value: vi.fn(() => 'blob:account-export'),
    });
    Object.defineProperty(URL, 'revokeObjectURL', {
      configurable: true,
      value: vi.fn(),
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

  it('downloads an account export only after the completion record arrives', async () => {
    useAuthStore.setState({
      user: { id: 'reader', email: 'reader@example.com', role: 'user' },
    });
    localStorage.setItem('auth_token', 'access');
    const exportBlob = new Blob([
      '{"type":"manifest","schema":"account-export-v1"}\n',
      '{"type":"complete","schema":"account-export-v1"}\n',
    ], { type: 'application/x-ndjson' });
    mocks.get.mockResolvedValueOnce({ data: exportBlob });
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});

    render(<MemoryRouter><SettingsPage /></MemoryRouter>);
    fireEvent.click(await screen.findByRole('button', { name: '导出账号数据' }));

    await waitFor(() => expect(URL.createObjectURL).toHaveBeenCalledWith(exportBlob));
    expect(mocks.get).toHaveBeenCalledWith('/account/export', {
      responseType: 'blob',
      timeout: 16 * 60 * 1000,
    });
    expect(click).toHaveBeenCalledOnce();
    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:account-export');
    expect(mocks.success).toHaveBeenCalledWith('账号数据导出完成');
    expect(useAuthStore.getState().user?.id).toBe('reader');
    expect(localStorage.getItem('auth_token')).toBe('access');
    click.mockRestore();
  });

  it('does not save or sign out after an incomplete account export', async () => {
    useAuthStore.setState({
      user: { id: 'reader', email: 'reader@example.com', role: 'user' },
    });
    localStorage.setItem('auth_token', 'access');
    mocks.get.mockResolvedValueOnce({
      data: new Blob(['{"type":"manifest","schema":"account-export-v1"}\n']),
    });

    render(<MemoryRouter><SettingsPage /></MemoryRouter>);
    fireEvent.click(await screen.findByRole('button', { name: '导出账号数据' }));

    await waitFor(() => expect(
      screen.getByRole('button', { name: '导出账号数据' }).hasAttribute('disabled'),
    ).toBe(false));
    expect(URL.createObjectURL).not.toHaveBeenCalled();
    expect(mocks.error).toHaveBeenCalledWith('账号导出未完整完成，请重试');
    expect(useAuthStore.getState().user?.id).toBe('reader');
    expect(localStorage.getItem('auth_token')).toBe('access');
  });
});
