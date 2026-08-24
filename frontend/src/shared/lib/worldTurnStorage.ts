export const worldTurnPendingStoragePrefix = 'novelworld:pending-world-turn:';

export function worldTurnPendingStorageKey(userId: string, novelId: string) {
  return `${worldTurnPendingStoragePrefix}${userId}:${novelId}`;
}

export function removeWorldTurnPendingRequest(userId: string, novelId: string) {
  if (typeof window === 'undefined') return;
  try {
    window.sessionStorage.removeItem(worldTurnPendingStorageKey(userId, novelId));
  } catch {
    // Restricted storage must not block a successful lifecycle transition.
  }
}

export function clearWorldTurnPendingRequests(keepUserId?: string) {
  if (typeof window === 'undefined') return;
  try {
    const keepPrefix = keepUserId
      ? `${worldTurnPendingStoragePrefix}${keepUserId}:`
      : undefined;
    const keys = Array.from(
      { length: window.sessionStorage.length },
      (_, index) => window.sessionStorage.key(index),
    ).filter((key): key is string => Boolean(
      key?.startsWith(worldTurnPendingStoragePrefix)
        && (!keepPrefix || !key.startsWith(keepPrefix)),
    ));
    keys.forEach(key => window.sessionStorage.removeItem(key));
  } catch {
    // Restricted storage must not block an authentication transition.
  }
}
