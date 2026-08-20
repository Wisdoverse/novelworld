import React from 'react';
import { render } from '@testing-library/react';
import { describe, it, vi } from 'vitest';
import { MemoryRouter } from 'react-router-dom';
import { expectNoA11yViolations } from '@/a11y';
import { ShelfPage } from './ShelfPage';

vi.mock('react-router-dom', async (importOriginal) => ({
  ...(await importOriginal<typeof import('react-router-dom')>()),
  useNavigate: () => vi.fn(),
}));
vi.mock('@/entities/novel/api', () => ({
  useNovels: () => ({
    data: [{
      id: 'novel', user_id: 'user', title: 'Portable novel', author: 'Portable author',
      status: 'ready', total_chapters: 3, cover_url: null, original_file_key: null,
      created_at: '2026-08-13T00:00:00Z',
    }],
    isLoading: false, refetch: vi.fn(),
  }),
  useImportNovel: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useUploadNovel: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useDeleteNovel: () => ({ mutate: vi.fn(), isPending: false }),
  useRetryNovel: () => ({ mutate: vi.fn(), isPending: false }),
  validateNovelFile: () => null,
}));
vi.mock('@/features/auth/model/useAuthStore', () => ({
  useAuthStore: () => ({ user: { id: 'user' } }),
}));
vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

describe('ShelfPage a11y', () => {
  it('has no axe violations with a novel card rendered', async () => {
    const { container } = render(
      <MemoryRouter><ShelfPage /></MemoryRouter>,
    );
    await expectNoA11yViolations(container);
  });
});