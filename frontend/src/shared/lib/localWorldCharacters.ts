import type { OpenWorldView } from '@/shared/types';

// A latest-turn event at the player's current location is the only encounter
// evidence this world contract provides. Older sightings are not current presence.
export function localWorldCharacterIds(view: OpenWorldView): Set<string> {
  const locationId = view.player?.location_id;
  const latest = view.journal?.[view.journal.length - 1];
  if (!locationId || !latest || latest.turn_number !== view.session?.turn_number) {
    return new Set();
  }

  const known = new Set(view.session.entry_context?.characters?.map(character => character.id) ?? []);
  const dead = new Set(view.session.dead_character_ids);
  return new Set(latest.transition.events
    .filter(event => event.location_id === locationId)
    .flatMap(event => event.actor_character_ids)
    .filter(id => known.has(id) && !dead.has(id)));
}
