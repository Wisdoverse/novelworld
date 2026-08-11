import { describe, expect, it } from 'vitest';
import type { Character } from '@/shared/types';
import { isCharacterAvailable } from './api';

const character = (firstAppearance?: number): Character => ({
  id: crypto.randomUUID(),
  novel_id: 'novel',
  name: 'Character',
  role: 'supporting',
  aliases: [],
  avatar_status: 'pending',
  first_appearance_chapter: firstAppearance,
});

describe('character visibility', () => {
  it('fails closed and follows forward and rewind progress', () => {
    expect(isCharacterAvailable(character(undefined), 5)).toBe(false);
    expect(isCharacterAvailable(character(0), 5)).toBe(false);
    expect(isCharacterAvailable(character(5), 4)).toBe(false);
    expect(isCharacterAvailable(character(5), 5)).toBe(true);
    expect(isCharacterAvailable(character(5), 1)).toBe(false);
  });
});
