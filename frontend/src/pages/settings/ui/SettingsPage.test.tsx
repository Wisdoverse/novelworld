import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SettingsPage } from './SettingsPage';

const mocks = vi.hoisted(() => ({ get: vi.fn(), put: vi.fn() }));

vi.mock('@/shared/api/client', () => ({
  apiClient: { get: mocks.get, put: mocks.put },
  getApiErrorMessage: () => '设置失败',
}));

describe('SettingsPage', () => {
  beforeEach(() => {
    mocks.get.mockReset();
    mocks.put.mockReset();
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
});
