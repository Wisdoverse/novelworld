import { useQuery } from '@tanstack/react-query';
import { apiClient } from '@/shared/api/client';

interface TranslationResponse {
  content: string;
}

export function useChapterTranslation(
  novelId: string,
  chapterNumber: number,
  content: string,
  enabled: boolean,
) {
  return useQuery({
    queryKey: ['chapter-translation', novelId, chapterNumber, content],
    queryFn: () => apiClient
      .post<TranslationResponse>(
        `/novels/${novelId}/chapters/${chapterNumber}/translation`,
        { content },
      )
      .then(response => response.data),
    enabled: enabled && Boolean(novelId) && chapterNumber > 0 && Boolean(content.trim()),
    staleTime: Infinity,
  });
}
