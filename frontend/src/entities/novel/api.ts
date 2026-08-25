import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { apiClient } from '@/shared/api/client';
import { removeWorldTurnPendingRequest } from '@/shared/lib/worldTurnStorage';
import type { Novel, Chapter, Character } from '@/shared/types';

// ─── Query Keys ───────────────────────────────────────────────────────────────
export const novelKeys = {
  all: ['novels'] as const,
  list: () => [...novelKeys.all, 'list'] as const,
  catalog: () => [...novelKeys.all, 'catalog'] as const,
  detail: (id: string) => [...novelKeys.all, 'detail', id] as const,
  chapters: (id: string) => [...novelKeys.all, id, 'chapters'] as const,
  chapter: (id: string, num: number) => [...novelKeys.all, id, 'chapters', num] as const,
  characters: (id: string, chapter: number) => [...novelKeys.all, id, 'characters', chapter] as const,
  status: (id: string) => [...novelKeys.all, id, 'status'] as const,
};

// ─── Hooks ────────────────────────────────────────────────────────────────────

export function shouldPollNovelList(novels: Novel[] | undefined) {
  return novels?.some(
    novel => novel.status === 'pending' || novel.status === 'parsing',
  ) ?? false;
}

export function useNovels() {
  return useQuery({
    queryKey: novelKeys.list(),
    queryFn: () => apiClient.get<Novel[]>('/novels').then(r => r.data),
    refetchInterval: (query) => shouldPollNovelList(query.state.data) ? 2000 : false,
  });
}

export function useNovelCatalog() {
  return useQuery({
    queryKey: novelKeys.catalog(),
    queryFn: () => apiClient.get<Novel[]>('/novels/catalog').then(r => r.data),
  });
}

export function useNovel(id: string) {
  return useQuery({
    queryKey: novelKeys.detail(id),
    queryFn: () => apiClient.get<Novel>(`/novels/${id}`).then(r => r.data),
    enabled: !!id,
  });
}

export function useNovelStatus(id: string, enabled = true) {
  return useQuery({
    queryKey: novelKeys.status(id),
    queryFn: () => apiClient.get<{ status: string; total_chapters: number; error?: string }>(
      `/novels/${id}/status`
    ).then(r => r.data),
    enabled: enabled && !!id,
    refetchInterval: (query) => {
      if (query.state.data?.status === 'parsing') return 2000;
      return false;
    },
  });
}

export function useChapters(novelId: string) {
  return useQuery({
    queryKey: novelKeys.chapters(novelId),
    queryFn: () => apiClient.get<Chapter[]>(`/novels/${novelId}/chapters`).then(r => r.data),
    enabled: !!novelId,
  });
}

export function useChapter(novelId: string, chapterNum: number) {
  return useQuery({
    queryKey: novelKeys.chapter(novelId, chapterNum),
    queryFn: () => apiClient.get<Chapter>(`/novels/${novelId}/chapters/${chapterNum}`).then(r => r.data),
    enabled: !!novelId && chapterNum > 0,
  });
}

export function isCharacterAvailable(character: Character, currentChapter: number) {
  const appearance = character.first_appearance_chapter;
  return appearance !== undefined && appearance >= 1 && appearance <= currentChapter;
}

export function useCharacters(novelId: string, currentChapter: number) {
  return useQuery({
    queryKey: novelKeys.characters(novelId, currentChapter),
    queryFn: () => apiClient
      .get<Character[]>(`/novels/${novelId}/characters`)
      .then(r => r.data.filter(character => isCharacterAvailable(character, currentChapter))),
    enabled: !!novelId && currentChapter >= 1,
  });
}

export function useImportNovel() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (data: {
      title: string;
      author?: string;
      content?: string;
      deviation_mode?: string;
    }) => apiClient.post<{ novel_id: string; status: string }>('/novels', data).then(r => r.data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: novelKeys.list() });
    },
  });
}

export interface NovelUploadInput {
  title: string;
  author?: string;
  deviationMode: string;
  file: File;
}

export interface NovelBatchUploadInput {
  author?: string;
  deviationMode: string;
  files: File[];
}

export interface NovelImportAccepted {
  novel_id: string;
  status: string;
}

export const MAX_NOVEL_BATCH_FILES = 5;
export const MAX_NOVEL_BATCH_BYTES = 40 * 1024 * 1024;

export function buildNovelUploadFormData(input: NovelUploadInput) {
  const form = new FormData();
  form.append('title', input.title);
  if (input.author) form.append('author', input.author);
  form.append('deviation_mode', input.deviationMode);
  form.append('file', input.file);
  return form;
}

export function buildNovelBatchUploadFormData(input: NovelBatchUploadInput) {
  const form = new FormData();
  if (input.author) form.append('author', input.author);
  form.append('deviation_mode', input.deviationMode);
  input.files.forEach(file => form.append('file', file));
  return form;
}

export function novelTitleFromFile(file: File) {
  return file.name.replace(/\.(txt|epub|pdf)$/i, '');
}

export function validateNovelFile(file: File): string | null {
  const extension = file.name.split('.').pop()?.toLowerCase();
  if (!extension || !['txt', 'epub', 'pdf'].includes(extension)) {
    return '请选择 TXT、EPUB 或 PDF 文件';
  }
  const limit = extension === 'txt' ? 10 * 1024 * 1024 : 20 * 1024 * 1024;
  if (file.size > limit) {
    return `${extension.toUpperCase()} 文件不能超过 ${limit / 1024 / 1024} MiB`;
  }
  return null;
}

export function validateNovelBatchFiles(files: File[]): string | null {
  if (!files.length) return '请至少选择一本小说';
  if (files.length > MAX_NOVEL_BATCH_FILES) {
    return `每次最多导入 ${MAX_NOVEL_BATCH_FILES} 本小说`;
  }
  for (const file of files) {
    const error = validateNovelFile(file);
    if (error) return `${file.name}：${error}`;
  }
  if (files.reduce((total, file) => total + file.size, 0) > MAX_NOVEL_BATCH_BYTES) {
    return '所选文件合计不能超过 40 MiB';
  }
  return null;
}

export function useUploadNovel() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: NovelUploadInput) => apiClient.post<{
      novel_id: string;
      status: string;
    }>('/novels/upload', buildNovelUploadFormData(input), {
      timeout: 60_000,
    }).then(r => r.data),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: novelKeys.list() }),
  });
}

export function useUploadNovelsBatch() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: NovelBatchUploadInput) => apiClient.post<{
      novels: NovelImportAccepted[];
      message: string;
    }>('/novels/upload/batch', buildNovelBatchUploadFormData(input), {
      timeout: 120_000,
    }).then(r => r.data),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: novelKeys.list() }),
  });
}

export function useDeleteNovel(userId?: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => apiClient.delete(`/novels/${id}`),
    onSuccess: (_response, novelId) => {
      if (userId) removeWorldTurnPendingRequest(userId, novelId);
      queryClient.invalidateQueries({ queryKey: novelKeys.list() });
      queryClient.invalidateQueries({ queryKey: novelKeys.catalog() });
    },
  });
}

export function useAttachNovel() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ novelId, deviationMode }: { novelId: string; deviationMode: string }) =>
      apiClient.post(`/novels/${novelId}/shelf`, { deviation_mode: deviationMode }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: novelKeys.list() });
      queryClient.invalidateQueries({ queryKey: novelKeys.catalog() });
    },
  });
}

export function useRetryNovel() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => apiClient.post(`/novels/${id}/retry`),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: novelKeys.list() }),
  });
}
