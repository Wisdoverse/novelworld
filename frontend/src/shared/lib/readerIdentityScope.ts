import type { ReadingProgress } from '@/shared/types';

export function getReaderIdentityScope(
  progress?: Pick<ReadingProgress, 'reader_identity_type' | 'reader_character_id'>,
) {
  if (progress?.reader_identity_type === 'self') return 'self';
  if (progress?.reader_identity_type === 'character' && progress.reader_character_id) {
    return `character:${progress.reader_character_id}`;
  }
  return 'unresolved';
}
