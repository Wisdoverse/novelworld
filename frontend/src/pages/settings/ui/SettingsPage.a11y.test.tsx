import React from 'react';
import { render } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, it, vi } from 'vitest';
import { expectNoA11yViolations } from '@/a11y';
import { SettingsPage } from './SettingsPage';
import { useAuthStore } from '@/features/auth/model/useAuthStore';

const mocks = vi.hoisted(() => ({ delete: vi.fn(), get: vi.fn(), put: vi.fn() }));
vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
vi.mock('@/shared/api/client', () => ({
  apiClient: { delete: mocks.delete, get: mocks.get, put: mocks.put },
  getApiErrorMessage: (_error: unknown, fallback: string) => fallback,
}));

describe('SettingsPage a11y', () => {
  beforeEach(() => {
    useAuthStore.setState({ user: { id: 'admin', email: 'admin@example.com', role: 'admin' }, loading: false });
    mocks.get.mockResolvedValue({
      data: { provider: 'deepseek', model: 'deepseek-v4-flash', thinking_enabled: false, api_key_configured: true },
    });
    Object.defineProperty(URL, 'createObjectURL', {
      configurable: true,
      value: vi.fn(() => 'blob:account-export'),
    });
  });
  it('has no axe violations', async () => {
    const { container } = render(
      <MemoryRouter><SettingsPage /></MemoryRouter>,
    );
    await expectNoA11yViolations(container);
  });
});