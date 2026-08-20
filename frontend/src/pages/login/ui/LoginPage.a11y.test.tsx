import React from 'react';
import { render } from '@testing-library/react';
import { describe, it, vi } from 'vitest';
import { MemoryRouter } from 'react-router-dom';
import { expectNoA11yViolations } from '@/a11y';
import { LoginPage } from './LoginPage';

vi.mock('react-router-dom', async (importOriginal) => ({
  ...(await importOriginal<typeof import('react-router-dom')>()),
  useNavigate: () => vi.fn(),
}));
vi.mock('@/features/auth/model/useAuthStore', () => ({
  useAuthStore: () => ({ login: vi.fn(), register: vi.fn(), loading: false }),
}));
vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

describe('LoginPage a11y', () => {
  it('has no axe violations on the login form', async () => {
    const { container } = render(
      <MemoryRouter><LoginPage /></MemoryRouter>,
    );
    await expectNoA11yViolations(container);
  });
});