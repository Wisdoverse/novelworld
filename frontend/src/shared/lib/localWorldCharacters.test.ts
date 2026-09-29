import { describe, expect, it } from 'vitest';
import type { OpenWorldView } from '@/shared/types';
import { localWorldCharacterIds } from './localWorldCharacters';

describe('localWorldCharacterIds', () => {
  it('accepts only living, known actors in the latest committed turn at the current place', () => {
    const view = {
      player: { location_id: 'gate' },
      session: {
        turn_number: 2,
        dead_character_ids: ['dead'],
        entry_context: { characters: [
          { id: 'near', name: '守门人' },
          { id: 'dead', name: '旧友' },
          { id: 'away', name: '信使' },
          { id: 'old', name: '旅人' },
        ] },
      },
      journal: [
        { turn_number: 1, transition: { events: [
          { location_id: 'gate', actor_character_ids: ['old'] },
        ] } },
        { turn_number: 2, transition: { events: [
          { location_id: 'gate', actor_character_ids: ['near', 'near', 'dead', 'unknown'] },
          { location_id: 'harbor', actor_character_ids: ['away'] },
          { location_id: null, actor_character_ids: ['old'] },
        ] } },
      ],
    } as unknown as OpenWorldView;

    expect([...localWorldCharacterIds(view)]).toEqual(['near']);
    expect(localWorldCharacterIds({ ...view, journal: view.journal.slice(0, 1) }).size).toBe(0);
    expect(localWorldCharacterIds({ ...view, player: { ...view.player, location_id: null } }).size).toBe(0);
  });
});
