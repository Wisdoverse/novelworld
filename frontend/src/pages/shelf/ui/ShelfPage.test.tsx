import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ShelfPage } from './ShelfPage';

const mocks = vi.hoisted(() => ({
  novels: [] as Array<Record<string, unknown>>,
  novelsError: false,
  novelsCachedOnError: false,
  catalog: [] as Array<Record<string, unknown>>,
  catalogError: false,
  catalogCachedOnError: false,
  refetchNovels: vi.fn(),
  refetchCatalog: vi.fn(),
  navigate: vi.fn(),
}));

vi.mock('react-router-dom', () => ({
  useNavigate: () => mocks.navigate,
}));

vi.mock('@/entities/novel', () => ({
  useNovels: () => ({
    data: mocks.novelsError && !mocks.novelsCachedOnError ? undefined : mocks.novels,
    isLoading: false,
    isError: mocks.novelsError,
    refetch: mocks.refetchNovels,
  }),
  useNovelCatalog: () => ({
    data: mocks.catalogError && !mocks.catalogCachedOnError ? undefined : mocks.catalog,
    isLoading: false,
    isError: mocks.catalogError,
    refetch: mocks.refetchCatalog,
  }),
  useDeleteNovel: () => ({ mutate: vi.fn() }),
  useRetryNovel: () => ({ mutate: vi.fn(), isPending: false, variables: undefined }),
  useAttachNovel: () => ({ mutateAsync: vi.fn(), isPending: false, variables: undefined }),
}));

vi.mock('@/features/auth', () => ({
  useAuthStore: (selector: (state: { user: { id: string } }) => unknown) => (
    selector({ user: { id: 'user' } })
  ),
}));

vi.mock('@/features/novel-import', () => ({
  NovelImportModal: () => <div role="dialog" aria-label="导入小说" />,
}));

vi.mock('@/shared/api/client', () => ({
  getApiErrorMessage: (_error: unknown, fallback: string) => fallback,
}));

vi.mock('sonner', () => ({
  toast: { success: vi.fn(), error: vi.fn() },
}));

describe('ShelfPage contracts', () => {
  beforeEach(() => {
    mocks.novels = [];
    mocks.novelsError = false;
    mocks.novelsCachedOnError = false;
    mocks.catalog = [];
    mocks.catalogError = false;
    mocks.catalogCachedOnError = false;
    mocks.refetchNovels.mockReset();
    mocks.refetchCatalog.mockReset();
    mocks.navigate.mockReset();
  });

  it('distinguishes a shelf query failure from an empty shelf and offers retry', () => {
    mocks.novelsError = true;
    render(<ShelfPage />);

    expect(screen.getByRole('heading', { name: '暂时无法加载书架' })).toBeTruthy();
    expect(screen.queryByRole('heading', { name: '书架还是空的' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: '重试' }));
    expect(mocks.refetchNovels).toHaveBeenCalledOnce();
  });

  it('keeps cached shelf data visible when a background refresh fails', () => {
    mocks.novels = [{
      id: 'novel',
      title: '故事',
      status: 'ready',
      total_chapters: 2,
      updated_at: '2026-01-01T00:00:00Z',
    }];
    mocks.novelsError = true;
    mocks.novelsCachedOnError = true;
    render(<ShelfPage />);

    expect(screen.getByRole('button', { name: '将 故事 移出书架' })).toBeTruthy();
    expect(screen.queryByRole('heading', { name: '暂时无法加载书架' })).toBeNull();
  });

  it('traps shared-library focus, closes with Escape, and restores the opener', async () => {
    render(<ShelfPage />);

    const opener = screen.getByRole('button', { name: '打开共享书库' });
    opener.focus();
    fireEvent.click(opener);
    const dialog = await screen.findByRole('dialog', { name: '共享书库' });
    const initialControl = screen.getByRole('button', { name: '忠实原著' });
    await waitFor(() => expect(document.activeElement).toBe(initialControl));

    opener.focus();
    await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true));

    fireEvent.keyDown(document, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog', { name: '共享书库' })).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(opener));
  });

  it('keeps the remove action visible and keyboard reachable on a ready novel', () => {
    mocks.novels = [{
      id: 'novel',
      title: '故事',
      status: 'ready',
      total_chapters: 2,
      updated_at: '2026-01-01T00:00:00Z',
    }];
    render(<ShelfPage />);

    const remove = screen.getByRole('button', { name: '将 故事 移出书架' });
    expect(remove.hasAttribute('disabled')).toBe(false);
    expect(remove.className).not.toContain('opacity-0');
  });
});
