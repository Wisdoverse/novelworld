import React from 'react';
import { render } from '@testing-library/react';
import { beforeEach, describe, it, vi } from 'vitest';
import { expectNoA11yViolations } from '@/a11y';
import { SetupPage } from './SetupPage';

const mocks = vi.hoisted(() => ({ post: vi.fn() }));
vi.mock('@/shared/api/client', () => ({
  apiClient: { post: mocks.post },
  getApiErrorMessage: () => 'Setup unavailable',
}));

describe('SetupPage a11y', () => {
  beforeEach(() => {
    mocks.post.mockReset();
    localStorage.clear();
  });
  it('has no axe violations', async () => {
    const { container } = render(<SetupPage onComplete={vi.fn()} llmConfigured={false} />);
    await expectNoA11yViolations(container);
  });
});