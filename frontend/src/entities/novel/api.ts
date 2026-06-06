import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { apiClient } from '@/shared/api/client';
import type { Novel, Chapter, Character } from '@/shared/types';

// ─── Query Keys ───────────────────────────────────────────────────────────────
export const novelKeys = {
  all: ['novels'] as const,
  list: () => [...novelKeys.all, 'list'] as const,
  detail: (id: string) => [...novelKeys.all, 'detail', id] as const,
  chapters: (id: string) => [...novelKeys.all, id, 'chapters'] as const,
  chapter: (id: string, num: number) => [...novelKeys.all, id, 'chapters', num] as const,
  characters: (id: string) => [...novelKeys.all, id, 'characters'] as const,
  status: (id: string) => [...novelKeys.all, id, 'status'] as const,
};

// ─── Hooks ────────────────────────────────────────────────────────────────────

export function useNovels() {
  return useQuery({
    queryKey: novelKeys.list(),
    queryFn: () => apiClient.get<Novel[]>('/novels').then(r => r.data),
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
    refetchInterval: (data) => {
      // 解析中每2秒轮询
      if (data?.status === 'parsing') return 2000;
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

export function useCharacters(novelId: string) {
  return useQuery({
    queryKey: novelKeys.characters(novelId),
    queryFn: () => apiClient.get<Character[]>(`/novels/${novelId}/characters`).then(r => r.data),
    enabled: !!novelId,
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

export function useDeleteNovel() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => apiClient.delete(`/novels/${id}`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: novelKeys.list() });
    },
  });
}
