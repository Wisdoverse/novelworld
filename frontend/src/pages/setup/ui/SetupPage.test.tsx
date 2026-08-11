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

  it('creates only the first administrator and stores returned tokens', async () => {
    mocks.post.mockResolvedValue({
      data: { access_token: 'access', refresh_token: 'refresh' },
    });
    const onComplete = vi.fn();
    render(<SetupPage onComplete={onComplete} />);

    fireEvent.change(screen.getByLabelText('Display name (optional)'), {
      target: { value: 'Admin' },
    });
    fireEvent.change(screen.getByLabelText('Email'), {
      target: { value: 'admin@test.invalid' },
    });
    fireEvent.change(screen.getByLabelText('Password'), {
      target: { value: 'password123' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Create administrator' }));

    await waitFor(() => expect(onComplete).toHaveBeenCalledOnce());
    expect(mocks.post).toHaveBeenCalledWith('/setup/init', {
      email: 'admin@test.invalid',
      password: 'password123',
      name: 'Admin',
      provider: 'runtime-configured',
      api_key: '',
    });
    expect(localStorage.getItem('auth_token')).toBe('access');
    expect(localStorage.getItem('refresh_token')).toBe('refresh');
    expect(screen.queryByLabelText(/API key/i)).toBeNull();
  });

  it('renders a stable inline error', async () => {
    mocks.post.mockRejectedValue(new Error('offline'));
    render(<SetupPage onComplete={vi.fn()} />);
    fireEvent.change(screen.getByLabelText('Email'), {
      target: { value: 'admin@test.invalid' },
    });
    fireEvent.change(screen.getByLabelText('Password'), {
      target: { value: 'password123' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Create administrator' }));

    expect((await screen.findByRole('alert')).textContent).toContain('Setup unavailable');
  });
});
