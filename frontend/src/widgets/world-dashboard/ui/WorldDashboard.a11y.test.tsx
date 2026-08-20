import React from 'react';
import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { OpenWorldView } from '@/shared/types';
import { expectNoA11yViolations } from '@/a11y';
import { WorldDashboard } from './WorldDashboard';

vi.mock('@/entities/narrative/api', () => ({
  useSubmitWorldTurn: () => ({ mutateAsync: vi.fn(), isPending: false }),
}));

const view = {
  player: {
    id: 'player', user_id: 'user', novel_id: 'novel', canonical_checkpoint_chapter: 1,
    name: '云舟', background: '地图学徒', capabilities: ['识图'], location_id: 'gate',
    inventory: [], relationships: {}, faction_standing: {}, discovered_knowledge: [],
    created_at: '2026-08-13T00:00:00Z',
  },
  session: {
    schema_version: 1, world_time: 1, turn_number: 1, dead_character_ids: [], character_perceptions: {},
    entry_context: {
      model_version: 1, checkpoint_chapter: 1, unlocked_through_chapter: 2,
      characters: [], locations: [{ id: 'gate', name: '旧城门' }], factions: [],
      hard_rules: [], dead_character_ids: [], threads: [{ id: 'siege', name: '围城' }],
      scheduled_events: [], character_goals: [],
    },
    canonical_events: [{
      id: 'siege-event', sequence: 1, summary: '围城开始', character_ids: [],
      location_ids: ['gate'], faction_ids: [], death_character_ids: [],
      source_chapters: [2], status: 'delayed', reason: '城门未开',
    }],
  },
  world_state: {
    user_id: 'user', novel_id: 'novel', updated_at: '2026-08-13T00:00:00Z',
    state: { choices: [], world_events: [], threads: { siege: { status: 'open', description: '围城', origin: 'canon' } } },
  },
  journal: [{
    turn_id: 'turn', turn_number: 1,
    action: { kind: 'investigate', target_id: 'siege', intent: '探查城门' },
    transition: {
      schema_version: 1, prompt_version: 'world-turn-v1', canon_model_version: 1,
      canonical_checkpoint_chapter: 1, rendered_narrative: '云舟发现守军换防。', events: [],
      relationship_changes: [], location_changes: [], thread_changes: [], player_location_id: null,
      inventory_additions: [], inventory_removals: [], knowledge_discoveries: [],
      faction_changes: [], canonical_event_change: null,
    },
    created_at: '2026-08-13T00:00:00Z', completed_at: '2026-08-13T00:00:01Z',
  }],
} as unknown as OpenWorldView;

describe('WorldDashboard a11y', () => {
  it('has no axe violations on the world dashboard', async () => {
    const { container } = render(<WorldDashboard novelId="novel" view={view} />);
    expect(screen.getByText(/围城开始/)).toBeTruthy();
    expect(screen.getByText(/云舟发现守军换防。/)).toBeTruthy();
    await expectNoA11yViolations(container);
  });
});