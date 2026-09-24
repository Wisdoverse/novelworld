import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { AxiosError, type AxiosResponse } from 'axios';
import { toast } from 'sonner';
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
  retryNovel: vi.fn(),
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
  useRetryNovel: () => ({ mutate: mocks.retryNovel, isPending: false, variables: undefined }),
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
    mocks.retryNovel.mockReset();
    mocks.navigate.mockReset();
    vi.mocked(toast.error).mockReset();
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

  it('shows ready books already on the shelf without allowing a second attachment', () => {
    mocks.novels = [{
      id: 'mine', title: '已加入', status: 'ready', total_chapters: 2,
      updated_at: '2026-01-01T00:00:00Z',
    }];
    mocks.catalog = [
      mocks.novels[0],
      { id: 'other', title: '新书', status: 'ready', total_chapters: 3 },
    ];
    render(<ShelfPage />);
    fireEvent.click(screen.getByRole('button', { name: '打开共享书库' }));

    expect(screen.getByRole('button', { name: '《已加入》已在书架' }).hasAttribute('disabled')).toBe(true);
    expect(screen.getByRole('button', { name: '将《新书》加入书架' }).hasAttribute('disabled')).toBe(false);
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

  it('uses safe guidance for known and unknown import failures and offers the matching action', () => {
    mocks.novels = [
      { id: 'missing', title: '文件丢失', status: 'error', parse_error: 'The retained source file is missing; re-upload the source', total_chapters: 0, updated_at: '2026-01-01T00:00:00Z' },
      { id: 'unknown', title: '未知错误', status: 'error', parse_error: 'token=private-secret', total_chapters: 0, updated_at: '2026-01-01T00:00:00Z' },
    ];
    render(<ShelfPage />);

    expect(screen.getByText('解析失败：原始文件已不可用，请重新导入小说。')).toBeTruthy();
    expect(screen.getByText('解析失败：具体原因未记录，可以尝试重试解析；若重试次数已用尽，请重新导入原始文件。')).toBeTruthy();
    expect(screen.queryByText(/private-secret/)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: '重新导入文件' }));
    expect(screen.getByRole('dialog', { name: '导入小说' })).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '重试解析' }));
    expect(mocks.retryNovel).toHaveBeenCalledWith('unknown', expect.objectContaining({ onSuccess: expect.any(Function), onError: expect.any(Function) }));
    const onError = mocks.retryNovel.mock.calls[0][1].onError as (error: AxiosError) => void;
    const retryResponse = (message: string) => new AxiosError('Retry failed', 'ERR_BAD_RESPONSE', undefined, undefined, {
      status: 409, data: { error: message },
    } as AxiosResponse);
    onError(retryResponse('Import provider budget exhausted; re-upload the source'));
    expect(toast.error).toHaveBeenLastCalledWith('重试次数已用尽，请重新导入原始文件。');
    onError(retryResponse('private provider response'));
    expect(toast.error).toHaveBeenLastCalledWith('重试失败，请稍后再试。');
  });
});
