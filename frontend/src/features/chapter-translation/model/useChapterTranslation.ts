import { useQuery } from '@tanstack/react-query';
import axios from 'axios';
import { apiClient } from '@/shared/api/client';

interface TranslationResponse {
  content: string;
}

// Covers the backend's four-minute ownership lease when a replica disappears.
const BUSY_RETRY_LIMIT = 55;
const STANDARD_RETRY_LIMIT = 3;
const MIN_RETRY_DELAY_MS = 1_000;
const MAX_RETRY_DELAY_MS = 5_000;
const TRANSLATION_REQUEST_TIMEOUT_MS = 190_000;

function isBusyResponse(error: unknown) {
  if (!axios.isAxiosError(error)) return false;
  return error.response?.status === 409 || error.response?.status === 429;
}

export function shouldRetryChapterTranslation(failureCount: number, error: unknown) {
  return failureCount < (isBusyResponse(error) ? BUSY_RETRY_LIMIT : STANDARD_RETRY_LIMIT);
}

export function chapterTranslationRetryDelay(failureCount: number, error: unknown) {
  if (isBusyResponse(error) && axios.isAxiosError(error)) {
    const retryAfter = Number(error.response?.headers?.['retry-after']);
    if (Number.isFinite(retryAfter)) {
      return Math.min(
        MAX_RETRY_DELAY_MS,
        Math.max(MIN_RETRY_DELAY_MS, retryAfter * 1_000),
      );
    }
    return MAX_RETRY_DELAY_MS;
  }

  return MIN_RETRY_DELAY_MS * 2 ** failureCount;
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
        { timeout: TRANSLATION_REQUEST_TIMEOUT_MS },
      )
      .then(response => response.data),
    enabled: enabled && Boolean(novelId) && chapterNumber > 0 && Boolean(content.trim()),
    staleTime: Infinity,
    retry: shouldRetryChapterTranslation,
    retryDelay: chapterTranslationRetryDelay,
  });
}
