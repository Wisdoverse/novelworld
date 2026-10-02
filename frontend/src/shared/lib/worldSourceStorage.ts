import type { WorldSourceAdvanceRequest } from '@/shared/types';

export const worldSourceStoragePrefix = 'novelworld:pending-world-source:';
export interface PendingWorldSource {
  idempotencyKey: string;
  request: WorldSourceAdvanceRequest;
  synchronizedChapter?: number;
  result?: { operation_id: string; previous_source_chapter: number; source_chapter: number };
  terminal?: boolean;
  notDispatched?: boolean;
}
export function worldSourceStorageKey(userId: string, novelId: string) {
  return `${worldSourceStoragePrefix}${userId}:${novelId}`;
}
export function readPendingWorldSource(userId: string | undefined, novelId: string): PendingWorldSource | null {
  if (!userId) return null;
  try {
    const value = window.sessionStorage.getItem(worldSourceStorageKey(userId, novelId));
    if (!value || value.length > 1_024) return null;
    const parsed = JSON.parse(value) as PendingWorldSource;
    const request = parsed.request;
    if (!/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(parsed.idempotencyKey)
      || !request || Object.keys(request).length !== 3
      || !Number.isSafeInteger(request.expected_turn_number) || request.expected_turn_number < 0
      || !Number.isSafeInteger(request.expected_source_chapter) || request.expected_source_chapter < 1
      || request.target_chapter !== request.expected_source_chapter + 1
      || (parsed.synchronizedChapter !== undefined && (!Number.isSafeInteger(parsed.synchronizedChapter)
        || parsed.synchronizedChapter < request.target_chapter))) return null;
    return parsed;
  } catch {
    return null;
  }
}
export function storePendingWorldSource(userId: string, novelId: string, pending: PendingWorldSource | null) {
  try {
    const key = worldSourceStorageKey(userId, novelId);
    if (pending) window.sessionStorage.setItem(key, JSON.stringify(pending));
    else window.sessionStorage.removeItem(key);
  } catch {
    // The in-memory lock still protects this mount when browser storage is unavailable.
  }
}
