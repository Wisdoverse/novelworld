import React, { type PropsWithChildren } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { apiClient } from '@/shared/api/client';
import { worldTurnPendingStorageKey } from '@/shared/lib/worldTurnStorage';
import type { Character } from '@/shared/types';
import {
  buildNovelBatchUploadFormData,
  buildNovelUploadFormData,
  isCharacterAvailable,
  novelTitleFromFile,
  shouldPollNovelList,
  useDeleteNovel,
  validateNovelBatchFiles,
  validateNovelFile,
} from './api';

const character = (firstAppearance?: number): Character => ({
  id: crypto.randomUUID(),
  novel_id: 'novel',
  name: 'Character',
  role: 'supporting',
  aliases: [],
  avatar_status: 'pending',
  first_appearance_chapter: firstAppearance,
});

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
      Array.from({ length: 6 }, (_, index) => new File(['x'], `${index}.txt`)),
    )).toContain('最多导入 5 本');
    expect(validateNovelBatchFiles([
      { name: 'one.epub', size: 20 * 1024 * 1024 } as File,
      { name: 'two.epub', size: 20 * 1024 * 1024 } as File,
      { name: 'three.txt', size: 1 } as File,
    ])).toContain('合计不能超过 40 MiB');
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

describe('character visibility', () => {
  it('fails closed and follows forward and rewind progress', () => {
    expect(isCharacterAvailable(character(undefined), 5)).toBe(false);
    expect(isCharacterAvailable(character(0), 5)).toBe(false);
    expect(isCharacterAvailable(character(5), 4)).toBe(false);
    expect(isCharacterAvailable(character(5), 5)).toBe(true);
    expect(isCharacterAvailable(character(5), 1)).toBe(false);
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
