import { beforeEach as beforeLocaleTest } from 'vitest';
import { setLocale } from '@/shared/lib/i18n';
import React, { type PropsWithChildren } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { apiClient } from '@/shared/api/client';
import { worldTurnPendingStorageKey } from '@/shared/lib/worldTurnStorage';
import type { WorldSeries } from '@/shared/types';
import {
  buildNovelBatchUploadFormData,
  splitNovelUploadBatches,
  buildNovelUploadFormData,
  novelKeys,
  novelTitleFromFile,
  sanitizeCharacterPersona,
  shouldPollNovelList,
  useCharacters,
  useDeleteNovel,
  useConfirmWorldSeriesBackground,
  useSuggestNovelWorldSeriesDeepSeek,
  useWorldSeriesList,
  useWorldSeriesContribution,
  useSetWorldSeriesContribution,
  useCommunitySeriesSuggestion,
  validateNovelBatchFiles,
  validateNovelFile,
} from './api';

describe('novel file uploads', () => {
  it('accepts TXT and EPUB within their size limits', () => {
    expect(validateNovelFile(new File(['text'], 'story.txt', { type: 'text/plain' }))).toBeNull();
    expect(validateNovelFile(new File(['epub'], 'story.EPUB', { type: 'application/epub+zip' }))).toBeNull();
    expect(validateNovelFile(new File(['x'], 'story.docx'))).toContain('TXT、EPUB');
  });

  it('builds the multipart upload contract without a client identity', () => {
    const file = new File(['story'], 'story.epub', { type: 'application/epub+zip' });
    const form = buildNovelUploadFormData({
      title: 'Story',
      author: 'Author',
      deviationMode: 'canon',
      file,
    });
    expect(form.get('title')).toBe('Story');
    expect(form.get('author')).toBe('Author');
    expect(form.get('deviation_mode')).toBe('canon');
    expect(form.get('file')).toBe(file);
    expect(form.has('user_id')).toBe(false);
  });

  it('builds one bounded multipart request for a file batch', () => {
    const files = [
      new File(['first'], 'first.txt', { type: 'text/plain' }),
      new File(['second'], 'second.pdf', { type: 'application/pdf' }),
    ];
    const form = buildNovelBatchUploadFormData({
      author: 'Shared author',
      deviationMode: 'creative',
      files,
    });

    expect(form.getAll('file')).toEqual(files);
    expect(form.get('author')).toBe('Shared author');
    expect(form.get('deviation_mode')).toBe('creative');
    expect(form.has('user_id')).toBe(false);
  });

  it('bounds a batch and derives titles from supported file names', () => {
    expect(novelTitleFromFile(new File([], 'The Story.EPUB'))).toBe('The Story');
    expect(validateNovelBatchFiles([
      new File(['one'], 'one.txt'),
      new File(['two'], 'two.pdf'),
    ])).toBeNull();
    expect(validateNovelBatchFiles(
      Array.from({ length: 51 }, (_, index) => new File(['x'], `${index}.txt`)),
    )).toContain('最多导入 50 本');
    expect(validateNovelBatchFiles([
      { name: 'one.epub', size: 20 * 1024 * 1024 } as File,
      { name: 'two.epub', size: 20 * 1024 * 1024 } as File,
      { name: 'three.txt', size: 1 } as File,
    ])).toBeNull();
  });
});

describe('upload batch planning', () => {
  it('splits 50 selected novels into ten requests, preserving order', () => {
    const files = Array.from({ length: 50 }, (_, index) => new File(['x'], `${index}.txt`));
    const batches = splitNovelUploadBatches(files);
    expect(batches.map(batch => batch.length)).toEqual(Array(10).fill(5));
    expect(batches.flat()).toEqual(files);
  });
  it('splits on the byte boundary independently of the file count', () => {
    const files = [20, 20, 1, 20].map((mib, index) => ({ name: `${index}.epub`, size: mib * 1024 * 1024 } as File));
    expect(splitNovelUploadBatches(files)).toEqual([files.slice(0, 2), files.slice(2)]);
  });
});

describe('novel ingestion status', () => {
  it('polls while background ingestion is pending or parsing', () => {
    const novel = (status: 'pending' | 'parsing' | 'ready' | 'error') => ({
      id: 'novel',
      user_id: 'user',
      title: 'Story',
      total_chapters: 0,
      status,
      deviation_mode: 'canon' as const,
      created_at: new Date(0).toISOString(),
      updated_at: new Date(0).toISOString(),
    });
    expect(shouldPollNovelList([novel('parsing')])).toBe(true);
    expect(shouldPollNovelList([novel('pending')])).toBe(true);
    expect(shouldPollNovelList([novel('ready')])).toBe(false);
    expect(shouldPollNovelList([novel('error')])).toBe(false);
  });
});

describe('character persona boundary', () => {
  const fullCharacter = {
    id: 'character',
    novel_id: 'novel',
    name: 'Character',
    aliases: ['Future Alias'],
    role: 'protagonist' as const,
    description: 'Future description',
    personality: 'Future personality',
    background: 'Future background',
    speaking_style: 'Future speaking style',
    appearance: 'Future appearance',
    avatar_url: 'https://example.invalid/future.png',
    avatar_status: 'ready' as const,
    first_appearance_chapter: 1,
  };

  it.each([
    ['missing', undefined],
    ['zero', 0],
    ['fractional', 1.5],
    ['ahead of progress', 3],
  ])('fails closed for a %s high-water marker', (_label, highWater) => {
    const sanitized = sanitizeCharacterPersona({
      ...fullCharacter,
      persona_source_chapter_high_water: highWater,
    }, 2);

    expect(sanitized).toBeNull();
  });

  it.each([
    ['missing', undefined],
    ['zero', 0],
    ['fractional', 1.5],
    ['ahead of progress', 3],
  ])('drops a character with a %s first appearance', (_label, firstAppearance) => {
    expect(sanitizeCharacterPersona({
      ...fullCharacter,
      first_appearance_chapter: firstAppearance,
      persona_source_chapter_high_water: 2,
    }, 2)).toBeNull();
  });

  it('allowlists a bounded full persona', () => {
    const full = { ...fullCharacter, persona_source_chapter_high_water: 2 };
    const response = {
      ...full,
      created_at: '2026-08-27T00:00:00Z',
      updated_at: '2026-08-27T00:00:00Z',
      system_prompt: 'never public',
      future_secret: 'future spoiler',
    };
    const sanitized = sanitizeCharacterPersona(response, 2);

    expect(sanitized).toEqual(full);
    expect(sanitized).not.toHaveProperty('created_at');
    expect(sanitized).not.toHaveProperty('updated_at');
    expect(sanitized).not.toHaveProperty('system_prompt');
    expect(sanitized).not.toHaveProperty('future_secret');
  });

  it('accepts only the exact four-field partial response', () => {
    const partial = {
      id: 'partial',
      novel_id: 'novel',
      name: 'Partial',
      first_appearance_chapter: 1,
    };
    const unknownPartial = { ...partial, future_secret: 'spoiler' };
    expect(sanitizeCharacterPersona(partial, 2)).toEqual(partial);
    expect(sanitizeCharacterPersona({ ...partial, aliases: ['Alias'] }, 2)).toBeNull();
    expect(sanitizeCharacterPersona(unknownPartial, 2)).toBeNull();
    expect(sanitizeCharacterPersona(fullCharacter, 2)).toBeNull();
    expect(sanitizeCharacterPersona(partial, 1.5)).toBeNull();
  });

  it('does not request while locked and filters stale complete responses after unlock', async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const wrapper = ({ children }: PropsWithChildren) => React.createElement(
      QueryClientProvider,
      { client: queryClient },
      children,
    );
    const partial = {
      id: 'partial',
      novel_id: 'novel',
      name: 'Partial',
      first_appearance_chapter: 1,
    };
    const get = vi.spyOn(apiClient, 'get').mockResolvedValue({
      data: [
        fullCharacter,
        { ...fullCharacter, persona_source_chapter_high_water: 5 },
        { ...partial, id: 'future', name: 'Future', first_appearance_chapter: 5 },
        { ...partial, id: 'alias-only', aliases: ['Future Alias'] },
        partial,
      ],
    });

    const { result, rerender } = renderHook(
      ({ enabled }) => useCharacters('novel', 2, enabled),
      { initialProps: { enabled: false }, wrapper },
    );

    expect(get).not.toHaveBeenCalled();
    expect(result.current.data).toBeUndefined();
    expect(queryClient.getQueryData(novelKeys.characters('novel', 2))).toBeUndefined();

    rerender({ enabled: true });
    await waitFor(() => expect(result.current.data).toEqual([partial]));
    expect(queryClient.getQueryData(novelKeys.characters('novel', 2))).toEqual([partial]);
    get.mockRestore();
  });
});

describe('novel lifecycle pending-turn cleanup', () => {
  let queryClient: QueryClient;
  let wrapper: ({ children }: PropsWithChildren) => React.ReactElement;

  beforeEach(() => {
    vi.restoreAllMocks();
    sessionStorage.clear();
    queryClient = new QueryClient({
      defaultOptions: { mutations: { retry: false }, queries: { retry: false } },
    });
    wrapper = ({ children }) => React.createElement(
      QueryClientProvider,
      { client: queryClient },
      children,
    );
  });

  it('removes only the deleted novel request after server success', async () => {
    vi.spyOn(apiClient, 'delete').mockResolvedValue({ data: undefined });
    const deleted = worldTurnPendingStorageKey('user-a', 'novel-a');
    const otherNovel = worldTurnPendingStorageKey('user-a', 'novel-b');
    const otherUser = worldTurnPendingStorageKey('user-b', 'novel-a');
    sessionStorage.setItem(deleted, 'private deleted intent');
    sessionStorage.setItem(otherNovel, 'keep other novel');
    sessionStorage.setItem(otherUser, 'keep other user');
    const { result } = renderHook(() => useDeleteNovel('user-a'), { wrapper });

    await act(async () => {
      await result.current.mutateAsync('novel-a');
    });

    expect(sessionStorage.getItem(deleted)).toBeNull();
    expect(sessionStorage.getItem(otherNovel)).toBe('keep other novel');
    expect(sessionStorage.getItem(otherUser)).toBe('keep other user');
  });

  it('retains exact recovery state when novel deletion fails', async () => {
    vi.spyOn(apiClient, 'delete').mockRejectedValue(new Error('delete unavailable'));
    const pendingKey = worldTurnPendingStorageKey('user-a', 'novel-a');
    sessionStorage.setItem(pendingKey, 'recoverable intent');
    const { result } = renderHook(() => useDeleteNovel('user-a'), { wrapper });

    await act(async () => {
      await expect(result.current.mutateAsync('novel-a')).rejects.toThrow('delete unavailable');
    });

    expect(sessionStorage.getItem(pendingKey)).toBe('recoverable intent');
  });
});

describe('principal-scoped world-series queries', () => {
  it('keeps contribution settings scoped to the principal and series', async () => {
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: PropsWithChildren) => React.createElement(QueryClientProvider, { client: queryClient }, children);
    const get = vi.spyOn(apiClient, 'get')
      .mockResolvedValueOnce({ data: { enabled: true } } as never)
      .mockResolvedValueOnce({ data: { enabled: false } } as never);
    const first = renderHook(() => useWorldSeriesContribution('reader-a', 'series-a'), { wrapper });
    const second = renderHook(() => useWorldSeriesContribution('reader-b', 'series-b'), { wrapper });
    await waitFor(() => expect(first.result.current.data).toEqual({ enabled: true }));
    await waitFor(() => expect(second.result.current.data).toEqual({ enabled: false }));
    expect(get).toHaveBeenCalledWith('/novels/world-series/series-a/contribution', { signal: expect.any(AbortSignal) });
    expect(queryClient.getQueryData(novelKeys.worldSeriesContribution('reader-a', 'series-a'))).toEqual({ enabled: true });
    expect(queryClient.getQueryData(novelKeys.worldSeriesContribution('reader-b', 'series-b'))).toEqual({ enabled: false });
    vi.restoreAllMocks();
  });

  it('writes explicit consent once and invalidates only its scoped query', async () => {
    const queryClient = new QueryClient({ defaultOptions: { mutations: { retry: true, retryDelay: 0 } } });
    const wrapper = ({ children }: PropsWithChildren) => React.createElement(QueryClientProvider, { client: queryClient }, children);
    queryClient.setQueryData(novelKeys.worldSeriesContribution('reader-b', 'series-a'), { enabled: false });
    queryClient.setQueryData(novelKeys.worldSeriesContribution('reader-a', 'series-a'), { enabled: false });
    const put = vi.spyOn(apiClient, 'put').mockResolvedValue({ data: { enabled: true } } as never);
    const { result } = renderHook(() => useSetWorldSeriesContribution('reader-a'), { wrapper });
    await act(async () => { await result.current.mutateAsync({ seriesId: 'series-a', enabled: true }); });
    expect(put).toHaveBeenCalledOnce();
    expect(put).toHaveBeenCalledWith('/novels/world-series/series-a/contribution', { enabled: true });
    expect(queryClient.getQueryState(novelKeys.worldSeriesContribution('reader-a', 'series-a'))?.isInvalidated).toBe(true);
    expect(queryClient.getQueryState(novelKeys.worldSeriesContribution('reader-b', 'series-a'))?.isInvalidated).toBe(false);
    expect(queryClient.getQueryData(novelKeys.worldSeriesContribution('reader-b', 'series-a'))).toEqual({ enabled: false });
    vi.restoreAllMocks();
  });

  it('does not recreate private consent cache from a late mutation after logout', async () => {
    const queryClient = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
    const wrapper = ({ children }: PropsWithChildren) => React.createElement(QueryClientProvider, { client: queryClient }, children);
    let resolvePut!: (value: { data: { enabled: boolean } }) => void;
    const response = new Promise<{ data: { enabled: boolean } }>(resolve => { resolvePut = resolve; });
    vi.spyOn(apiClient, 'put').mockReturnValue(response as never);
    const key = novelKeys.worldSeriesContribution('reader-a', 'series-a');
    queryClient.setQueryData(key, { enabled: false });
    const { result } = renderHook(() => useSetWorldSeriesContribution('reader-a'), { wrapper });
    let pending!: Promise<{ enabled: boolean }>;
    act(() => { pending = result.current.mutateAsync({ seriesId: 'series-a', enabled: true }); });
    await waitFor(() => expect(apiClient.put).toHaveBeenCalledOnce());
    queryClient.clear();
    await act(async () => { resolvePut({ data: { enabled: true } }); await pending; });
    expect(queryClient.getQueryData(key)).toBeUndefined();
    expect(queryClient.getQueryState(key)).toBeUndefined();
    vi.restoreAllMocks();
  });

  it('queries community evidence without retrying or invoking a provider endpoint', async () => {
    const queryClient = new QueryClient({ defaultOptions: { mutations: { retry: true, retryDelay: 0 } } });
    const wrapper = ({ children }: PropsWithChildren) => React.createElement(QueryClientProvider, { client: queryClient }, children);
    const post = vi.spyOn(apiClient, 'post').mockRejectedValue(new Error('unavailable'));
    const { result } = renderHook(() => useCommunitySeriesSuggestion(), { wrapper });
    await act(async () => { await expect(result.current.mutateAsync('novel-1')).rejects.toThrow('unavailable'); });
    expect(post).toHaveBeenCalledOnce();
    expect(post).toHaveBeenCalledWith('/novels/novel-1/world-series/community-suggestion');
    vi.restoreAllMocks();
  });

  it('confirms a background once and invalidates the list and every associated novel', async () => {
    const queryClient = new QueryClient({ defaultOptions: { mutations: { retry: true, retryDelay: 0 } } });
    const wrapper = ({ children }: PropsWithChildren) => React.createElement(
      QueryClientProvider, { client: queryClient }, children,
    );
    const series = { id: 'series-1', name: 'Series', background: 'Confirmed', source_novel_id: 'source-1' } as WorldSeries;
    queryClient.setQueryData(novelKeys.worldSeriesList('reader-1'), [series]);
    queryClient.setQueryData(novelKeys.novelWorldSeries('reader-1', 'novel-1'), series);
    queryClient.setQueryData(novelKeys.novelWorldSeries('reader-1', 'novel-2'), series);
    queryClient.setQueryData(novelKeys.novelWorldSeries('reader-2', 'novel-3'), series);
    const put = vi.spyOn(apiClient, 'put').mockResolvedValue({ data: series } as never);
    const { result } = renderHook(() => useConfirmWorldSeriesBackground('reader-1'), { wrapper });

    await act(async () => {
      await result.current.mutateAsync({ seriesId: 'series-1', background: 'Confirmed' });
    });

    expect(put).toHaveBeenCalledOnce();
    expect(put).toHaveBeenCalledWith('/novels/world-series/series-1/background', { background: 'Confirmed' });
    expect(queryClient.getQueryState(novelKeys.worldSeriesList('reader-1'))?.isInvalidated).toBe(true);
    expect(queryClient.getQueryState(novelKeys.novelWorldSeries('reader-1', 'novel-1'))?.isInvalidated).toBe(true);
    expect(queryClient.getQueryState(novelKeys.novelWorldSeries('reader-1', 'novel-2'))?.isInvalidated).toBe(true);
    expect(queryClient.getQueryState(novelKeys.novelWorldSeries('reader-2', 'novel-3'))?.isInvalidated).toBe(false);
    vi.restoreAllMocks();
  });

  it('keeps a late prior-principal response under its own cache key', async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const wrapper = ({ children }: PropsWithChildren) => React.createElement(
      QueryClientProvider,
      { client: queryClient },
      children,
    );
    let resolveFirst!: (response: { data: WorldSeries[] }) => void;
    const firstResponse = new Promise<{ data: WorldSeries[] }>(resolve => { resolveFirst = resolve; });
    const first = [{ id: 'series-a', name: 'A' }] as WorldSeries[];
    const second = [{ id: 'series-b', name: 'B' }] as WorldSeries[];
    vi.spyOn(apiClient, 'get')
      .mockReturnValueOnce(firstResponse as never)
      .mockResolvedValueOnce({ data: second } as never);

    const previousPrincipal = renderHook(() => useWorldSeriesList('reader-a'), { wrapper });
    const currentPrincipal = renderHook(() => useWorldSeriesList('reader-b'), { wrapper });
    await waitFor(() => expect(currentPrincipal.result.current.data).toEqual(second));
    resolveFirst({ data: first });
    await waitFor(() => expect(previousPrincipal.result.current.data).toEqual(first));

    expect(queryClient.getQueryData(novelKeys.worldSeriesList('reader-a'))).toEqual(first);
    expect(queryClient.getQueryData(novelKeys.worldSeriesList('reader-b'))).toEqual(second);
    vi.restoreAllMocks();
  });

  it('calls the explicit DeepSeek supplement route once without retrying', async () => {
    const queryClient = new QueryClient({
      defaultOptions: { mutations: { retry: true, retryDelay: 0 } },
    });
    const wrapper = ({ children }: PropsWithChildren) => React.createElement(
      QueryClientProvider,
      { client: queryClient },
      children,
    );
    const post = vi.spyOn(apiClient, 'post').mockRejectedValue(new Error('temporary failure'));
    const { result } = renderHook(() => useSuggestNovelWorldSeriesDeepSeek(), { wrapper });

    await act(async () => {
      await expect(result.current.mutateAsync({ novelId: 'novel-42' })).rejects.toThrow('temporary failure');
    });

    expect(post).toHaveBeenCalledOnce();
    expect(post).toHaveBeenCalledWith(
      '/novels/novel-42/world-series/suggestion/deepseek',
      undefined,
      { timeout: 60_000 },
    );
    vi.restoreAllMocks();
  });

  it('uses read-only check mode for an explicit result query', async () => {
    const queryClient = new QueryClient({
      defaultOptions: { mutations: { retry: false } },
    });
    const wrapper = ({ children }: PropsWithChildren) => React.createElement(
      QueryClientProvider,
      { client: queryClient },
      children,
    );
    const response = { status: 'unknown_outcome', data: undefined };
    const post = vi.spyOn(apiClient, 'post').mockResolvedValue({ data: response } as never);
    const { result } = renderHook(() => useSuggestNovelWorldSeriesDeepSeek(), { wrapper });

    await act(async () => {
      await result.current.mutateAsync({ novelId: 'novel-42', checkOnly: true });
    });

    expect(post).toHaveBeenCalledWith(
      '/novels/novel-42/world-series/suggestion/deepseek',
      undefined,
      { timeout: 60_000, params: { check_only: true } },
    );
    vi.restoreAllMocks();
  });
});

// This suite retains the Simplified Chinese journey; locale tests cover the English default.
beforeLocaleTest(() => setLocale('zh-CN'));
