import { useSyncExternalStore } from 'react';

const media = typeof window === 'undefined'
  ? undefined : window.matchMedia?.('(prefers-reduced-motion: reduce)');

export const prefersReducedMotion = () => media?.matches ?? false;

function subscribe(onChange: () => void) {
  media?.addEventListener('change', onChange);
  return () => media?.removeEventListener('change', onChange);
}

export function useReducedMotionPreference() {
  return useSyncExternalStore(subscribe, prefersReducedMotion, () => false);
}
