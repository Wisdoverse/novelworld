import type { WorldSession } from '@/shared/types';

export function effectiveWorldContext(session: WorldSession) {
  return session.source_context ?? session.entry_context;
}
