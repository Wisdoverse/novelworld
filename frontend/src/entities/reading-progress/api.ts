import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { apiClient } from '@/shared/api/client';
import type { ReadingProgress } from '@/shared/types';

export const readingProgressKeys = {
  detail: (novelId: string) => ['reading-progress', novelId] as const,
};

export function useReadingProgress(novelId: string) {
  return useQuery({
    queryKey: readingProgressKeys.detail(novelId),
    queryFn: () => apiClient.get<ReadingProgress>(`/progress/${novelId}`).then((response) => response.data),
    enabled: !!novelId,
  });
}

export function useUpdateReadingProgress(novelId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (currentChapter: number) =>
      apiClient.put(`/progress/${novelId}`, { current_chapter: currentChapter }),
    onSuccess: () => queryClient.refetchQueries({
      queryKey: readingProgressKeys.detail(novelId),
      type: 'active',
    }),
  });
}
