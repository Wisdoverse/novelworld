import React from 'react';
import { render } from '@testing-library/react';
import { describe, it, vi } from 'vitest';
import type { OpenWorldView } from '@/shared/types';
import { expectNoA11yViolations } from '@/a11y';
import { WorldActionForm } from './WorldActionForm';

const view = {
  player: { name: '云舟', location_id: 'gate' },
  session: {
    entry_context: {
      locations: [{ id: 'gate', name: '旧城门' }],
      characters: [{ id: 'character', name: '守门人' }],
      dead_character_ids: [],
      character_goals: [{ id: 'canon-goal', character_id: 'character', description: '守住城门', source_chapters: [1] }],
    },
    canonical_events: [],
    dead_character_ids: [],
  },
  world_state: { state: { threads: {} } },
} as unknown as OpenWorldView;

describe('WorldActionForm a11y', () => {
  it('has no axe violations on the action form', async () => {
    const { container } = render(
      <WorldActionForm view={view} isPending={false} onSubmit={vi.fn()} />,
    );
    await expectNoA11yViolations(container);
  });
});