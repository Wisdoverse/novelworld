import { describe, expect, it } from 'vitest';
import { getReaderIdentityScope } from './readerIdentityScope';

describe('getReaderIdentityScope', () => {
  it('uses only durable character identity provenance', () => {
    expect(getReaderIdentityScope({
      reader_identity_type: 'character',
      reader_character_id: 'character-a',
    })).toBe('character:character-a');
  });

  it('fails closed when a character identity has no character id', () => {
    expect(getReaderIdentityScope({
      reader_identity_type: 'character',
      reader_character_id: undefined,
    })).toBe('unresolved');
  });

  it('keeps self mode distinct', () => {
    expect(getReaderIdentityScope({
      reader_identity_type: 'self',
      reader_character_id: undefined,
    })).toBe('self');
  });
});
