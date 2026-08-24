import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SettingsPage } from './SettingsPage';
import { useAuthStore } from '@/features/auth';
import { queryClient } from '@/shared/api/queryClient';

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
  getApiErrorCode: (error: { code?: string }) => error?.code,
  getApiErrorMessage: (_error: unknown, fallback: string) => fallback,
}));

vi.mock('@/features/llm-usage', () => ({
  LlmUsageCard: ({ principalId, scope }: { principalId: string; scope: string }) => (
    <div>{scope} usage for {principalId}</div>
  ),
}));

const settingsForCurrentUser = () => ({
  scope: 'platform',
  provider: 'deepseek',
  model: 'deepseek-v4-flash',
  thinking_enabled: false,
  api_key_configured: useAuthStore.getState().user?.role === 'admin',
});

describe('SettingsPage', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    mocks.get.mockReset();
    mocks.put.mockReset();
    mocks.delete.mockReset();
    mocks.error.mockReset();
    mocks.success.mockReset();
    queryClient.clear();
    localStorage.clear();
    sessionStorage.clear();
    useAuthStore.setState({
      user: { id: 'admin', email: 'admin@example.com', role: 'admin' },
      loading: false,
    });
    mocks.get.mockImplementation((url: string) => {
      if (url === '/settings/llm') return Promise.resolve({ data: settingsForCurrentUser() });
      return Promise.reject(new Error(`unexpected GET ${url}`));
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
        scope: 'platform',
        provider: 'deepseek',
        model: 'deepseek-v4-pro',
        thinking_enabled: true,
        api_key_configured: true,
      },
    });
    render(<MemoryRouter><SettingsPage /></MemoryRouter>);

    await screen.findByRole('heading', { name: '平台模型设置' });
    expect(screen.getByText('platform usage for admin')).toBeTruthy();
    fireEvent.change(screen.getByLabelText('模型'), { target: { value: 'deepseek-v4-pro' } });
    fireEvent.click(screen.getByRole('checkbox'));
    fireEvent.click(screen.getByRole('button', { name: '保存平台设置' }));

    await waitFor(() => expect(mocks.put).toHaveBeenCalledWith('/settings/llm', {
      provider: 'deepseek',
      model: 'deepseek-v4-pro',
      thinking_enabled: true,
      api_key: undefined,
    }));
  });

  it('lets the first administrator create platform settings after deferred setup', async () => {
    mocks.get.mockRejectedValueOnce({ code: 'setup_required' });
    mocks.put.mockResolvedValue({
      data: {
        scope: 'platform',
        provider: 'deepseek',
        model: 'deepseek-v4-flash',
        thinking_enabled: false,
        api_key_configured: true,
      },
    });
    render(<MemoryRouter><SettingsPage /></MemoryRouter>);

    await screen.findByRole('heading', { name: '平台模型设置' });
    const key = screen.getByLabelText('平台 API Key');
    expect(key).toHaveAttribute('required');
    fireEvent.change(key, { target: { value: 'first-platform-key' } });
    fireEvent.click(screen.getByRole('button', { name: '保存平台设置' }));

    await waitFor(() => expect(mocks.put).toHaveBeenCalledWith('/settings/llm', {
      provider: 'deepseek',
      model: 'deepseek-v4-flash',
      thinking_enabled: false,
      api_key: 'first-platform-key',
    }));
  });

  it('offers and saves the DeepSeek V4 Flash Vision experimental model', async () => {
    mocks.put.mockResolvedValue({
      data: {
        scope: 'platform',
        provider: 'deepseek',
        model: 'deepseek-v4-flash-vision-exp',
        thinking_enabled: false,
        api_key_configured: true,
      },
    });
    render(<MemoryRouter><SettingsPage /></MemoryRouter>);

    await screen.findByRole('heading', { name: '平台模型设置' });
    fireEvent.change(screen.getByLabelText('模型'), {
      target: { value: 'deepseek-v4-flash-vision-exp' },
    });
    fireEvent.click(screen.getByRole('button', { name: '保存平台设置' }));

    await waitFor(() => expect(mocks.put).toHaveBeenCalledWith('/settings/llm', {
      provider: 'deepseek',
      model: 'deepseek-v4-flash-vision-exp',
      thinking_enabled: false,
      api_key: undefined,
    }));
  });

  it('requires a personal key and hides usage until a reader configures one', async () => {
    useAuthStore.setState({
      user: { id: 'reader', email: 'reader@example.com', role: 'user' },
    });

    render(<MemoryRouter><SettingsPage /></MemoryRouter>);

    await screen.findByRole('heading', { name: '个人模型设置' });
    const keyInput = screen.getByLabelText('个人 API Key');
    expect(keyInput.hasAttribute('required')).toBe(true);
    expect(screen.queryByText(/usage for reader/)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: '保存个人设置' }));
    expect(mocks.put).not.toHaveBeenCalled();
  });

  it('rejects whitespace when a reader configures a personal key for the first time', async () => {
    useAuthStore.setState({
      user: { id: 'reader', email: 'reader@example.com', role: 'user' },
    });

    render(<MemoryRouter><SettingsPage /></MemoryRouter>);

    const keyInput = await screen.findByLabelText('个人 API Key');
    fireEvent.change(keyInput, { target: { value: '   ' } });
    fireEvent.click(screen.getByRole('button', { name: '保存个人设置' }));

    expect(mocks.put).not.toHaveBeenCalled();
    expect(mocks.error).toHaveBeenCalledWith('请输入个人 API Key');
  });

  it('clears an unsaved personal key when the signed-in principal changes', async () => {
    useAuthStore.setState({
      user: { id: 'user-a', email: 'a@example.com', role: 'user' },
    });

    render(<MemoryRouter><SettingsPage /></MemoryRouter>);

    const keyInput = await screen.findByLabelText('个人 API Key');
    fireEvent.change(keyInput, { target: { value: 'sk-user-a' } });
    expect((keyInput as HTMLInputElement).value).toBe('sk-user-a');

    useAuthStore.setState({
      user: { id: 'user-b', email: 'b@example.com', role: 'user' },
    });

    await waitFor(() => expect(
      (screen.getByLabelText('个人 API Key') as HTMLInputElement).value,
    ).toBe(''));
    expect(mocks.put).not.toHaveBeenCalled();
  });

  it('shows personal usage when the reader already has a personal key', async () => {
    useAuthStore.setState({
      user: { id: 'reader', email: 'reader@example.com', role: 'user' },
    });
    mocks.get.mockResolvedValue({
      data: {
        scope: 'user',
        provider: 'deepseek',
        model: 'deepseek-v4-flash',
        thinking_enabled: false,
        api_key_configured: true,
      },
    });

    render(<MemoryRouter><SettingsPage /></MemoryRouter>);

    await screen.findByRole('heading', { name: '个人模型设置' });
    expect(screen.getByText('user usage for reader')).toBeTruthy();
    expect(screen.getByLabelText(/个人 API Key/).hasAttribute('required')).toBe(false);
  });

  it('offers a safe personal default when the platform fallback is environment-managed', async () => {
    useAuthStore.setState({
      user: { id: 'reader', email: 'reader@example.com', role: 'user' },
    });
    mocks.get.mockResolvedValue({
      data: {
        scope: 'platform',
        provider: 'environment',
        model: 'operator-model',
        thinking_enabled: true,
        api_key_configured: false,
      },
    });

    render(<MemoryRouter><SettingsPage /></MemoryRouter>);

    await screen.findByRole('heading', { name: '个人模型设置' });
    expect((screen.getByLabelText('模型') as HTMLSelectElement).value).toBe('deepseek-v4-flash');
    expect(screen.getByLabelText('个人 API Key').hasAttribute('required')).toBe(true);
    expect(screen.queryByText(/usage for reader/)).toBeNull();
  });

  it('renders environment-managed platform settings read-only for an administrator', async () => {
    mocks.get.mockResolvedValue({
      data: {
        scope: 'platform',
        provider: 'environment',
        model: 'operator-model',
        thinking_enabled: false,
        api_key_configured: true,
      },
    });

    render(<MemoryRouter><SettingsPage /></MemoryRouter>);

    await screen.findByRole('heading', { name: '平台模型设置' });
    expect(screen.getByText('平台模型由环境变量管理')).toBeTruthy();
    expect(screen.getByText(/当前模型：operator-model/)).toBeTruthy();
    expect(screen.queryByRole('button', { name: '保存平台设置' })).toBeNull();
    expect(screen.getByText('platform usage for admin')).toBeTruthy();
  });

  it('shows usage and invalidates only that principal scope after saving a personal key', async () => {
    useAuthStore.setState({
      user: { id: 'reader', email: 'reader@example.com', role: 'user' },
    });
    mocks.put.mockResolvedValue({
      data: {
        scope: 'user',
        provider: 'deepseek',
        model: 'deepseek-v4-flash',
        thinking_enabled: false,
        api_key_configured: true,
      },
    });
    const invalidate = vi.spyOn(queryClient, 'invalidateQueries').mockResolvedValue();

    render(<MemoryRouter><SettingsPage /></MemoryRouter>);

    await screen.findByRole('heading', { name: '个人模型设置' });
    fireEvent.change(screen.getByLabelText('个人 API Key'), {
      target: { value: '  sk-reader  ' },
    });
    fireEvent.click(screen.getByRole('button', { name: '保存个人设置' }));

    await waitFor(() => expect(mocks.put).toHaveBeenCalledWith('/settings/llm', {
      provider: 'deepseek',
      model: 'deepseek-v4-flash',
      thinking_enabled: false,
      api_key: 'sk-reader',
    }));
    expect(await screen.findByText('user usage for reader')).toBeTruthy();
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: ['llm-usage', 'reader', 'user'],
      exact: true,
    });
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
    expect(mocks.get).toHaveBeenCalledWith('/settings/llm');
    expect(localStorage.getItem('auth_token')).toBeNull();
    confirm.mockRestore();
  });

  it('keeps account erasure available when administrator model settings fail', async () => {
    mocks.get.mockRejectedValueOnce(new Error('model settings unavailable'));

    render(<MemoryRouter><SettingsPage /></MemoryRouter>);

    expect(await screen.findByRole('heading', { name: '模型设置暂时不可用' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '删除账号' })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '重试' }));
    expect(await screen.findByRole('heading', { name: '平台模型设置' })).toBeTruthy();
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
    mocks.get.mockImplementation((url: string) => {
      if (url === '/settings/llm') return Promise.resolve({ data: settingsForCurrentUser() });
      if (url === '/account/export') return Promise.resolve({ data: exportBlob });
      return Promise.reject(new Error(`unexpected GET ${url}`));
    });
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
    const incompleteExport = new Blob(['{"type":"manifest","schema":"account-export-v1"}\n']);
    mocks.get.mockImplementation((url: string) => {
      if (url === '/settings/llm') return Promise.resolve({ data: settingsForCurrentUser() });
      if (url === '/account/export') return Promise.resolve({ data: incompleteExport });
      return Promise.reject(new Error(`unexpected GET ${url}`));
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

  it('discards a completed A export after principal B logs in', async () => {
    let resolveExport!: (value: unknown) => void;
    useAuthStore.setState({
      user: { id: 'user-a', email: 'a@example.com', role: 'user' },
    });
    localStorage.setItem('auth_token', 'access-a');
    mocks.get.mockImplementation((url: string) => {
      if (url === '/settings/llm') return Promise.resolve({ data: settingsForCurrentUser() });
      if (url === '/account/export') {
        return new Promise(resolve => { resolveExport = resolve; });
      }
      return Promise.reject(new Error(`unexpected GET ${url}`));
    });
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});

    render(<MemoryRouter><SettingsPage /></MemoryRouter>);
    fireEvent.click(await screen.findByRole('button', { name: '导出账号数据' }));
    await waitFor(() => expect(mocks.get).toHaveBeenCalledWith('/account/export', {
      responseType: 'blob',
      timeout: 16 * 60 * 1000,
    }));
    localStorage.setItem('auth_token', 'access-b');
    useAuthStore.setState({
      user: { id: 'user-b', email: 'b@example.com', role: 'user' },
    });
    resolveExport({
      data: new Blob([
        '{"type":"manifest","schema":"account-export-v1"}\n',
        '{"type":"complete","schema":"account-export-v1"}\n',
      ]),
    });

    await waitFor(() => expect(
      screen.getByRole('button', { name: '导出账号数据' }).hasAttribute('disabled'),
    ).toBe(false));
    expect(URL.createObjectURL).not.toHaveBeenCalled();
    expect(click).not.toHaveBeenCalled();
    expect(mocks.success).not.toHaveBeenCalled();
    expect(mocks.error).not.toHaveBeenCalled();
    expect(useAuthStore.getState().user?.id).toBe('user-b');
    expect(localStorage.getItem('auth_token')).toBe('access-b');
    click.mockRestore();
  });

  it('does not toast or navigate when an A deletion completes after B logs in', async () => {
    let resolveDelete!: (value: unknown) => void;
    useAuthStore.setState({
      user: { id: 'user-a', email: 'a@example.com', role: 'user' },
    });
    localStorage.setItem('auth_token', 'access-a');
    localStorage.setItem('refresh_token', 'refresh-a');
    mocks.delete.mockImplementationOnce(
      () => new Promise(resolve => { resolveDelete = resolve; }),
    );
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);

    render(<MemoryRouter><SettingsPage /></MemoryRouter>);
    fireEvent.click(await screen.findByRole('button', { name: '删除账号' }));
    await waitFor(() => expect(mocks.delete).toHaveBeenCalledWith('/auth/me'));
    localStorage.setItem('auth_token', 'access-b');
    localStorage.setItem('refresh_token', 'refresh-b');
    useAuthStore.setState({
      user: { id: 'user-b', email: 'b@example.com', role: 'user' },
    });
    resolveDelete({ data: undefined });

    await waitFor(() => expect(
      screen.getByRole('button', { name: '删除账号' }).hasAttribute('disabled'),
    ).toBe(false));
    expect(mocks.success).not.toHaveBeenCalled();
    expect(useAuthStore.getState().user?.id).toBe('user-b');
    expect(localStorage.getItem('auth_token')).toBe('access-b');
    expect(localStorage.getItem('refresh_token')).toBe('refresh-b');
    confirm.mockRestore();
  });
});
