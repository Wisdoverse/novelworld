import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { OpenWorldView } from '@/shared/types';
import { WorldDashboard } from './WorldDashboard';

const mocks = vi.hoisted(() => ({
  submit: vi.fn(),
}));

vi.mock('@/entities/narrative/api', () => ({
  useSubmitWorldTurn: () => ({ mutateAsync: mocks.submit, isPending: false }),
  isWorldTurnOutcomeUnknown: (error: { outcomeUnknown?: boolean }) => error.outcomeUnknown !== false,
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
      scheduled_events: [{ id: 'siege-event', sequence: 1, summary: '围城开始', character_ids: [], location_ids: ['gate'], faction_ids: [], death_character_ids: [], source_chapters: [2] }],
      character_goals: [],
    },
    canonical_events: [{ id: 'siege-event', sequence: 1, summary: '围城开始', character_ids: [], location_ids: ['gate'], faction_ids: [], death_character_ids: [], source_chapters: [2], status: 'delayed', reason: '城门未开' }],
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
} satisfies OpenWorldView;

describe('WorldDashboard', () => {
  beforeEach(() => {
    mocks.submit.mockReset();
  });

  it('keeps canon provenance distinct and retries a failed turn with the same key', async () => {
    mocks.submit.mockRejectedValue(new Error('offline'));
    render(<WorldDashboard novelId="novel" view={view} />);

    expect(screen.getAllByText(/原著主线/).length).toBeGreaterThan(0);
    expect(screen.getByText(/来源章节 2/)).toBeTruthy();
    // The journal distinguishes the reader action from the generated prose.
    expect(screen.getByText(/回合 1/)).toBeTruthy();
    expect(screen.getByText(/读者行动/)).toBeTruthy();
    expect(screen.getByText(/调查线索：探查城门/)).toBeTruthy();
    expect(screen.getByText(/生成叙事/)).toBeTruthy();
    expect(screen.getByText(/云舟发现守军换防。/)).toBeTruthy();
    expect(screen.getByText(/2026-08-13T00:00:01Z/)).toBeTruthy();

    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '前往城门' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));
    await waitFor(() => expect(mocks.submit).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole('button', { name: '放弃此请求' })).toBeNull();
    expect(screen.getByRole('alert').textContent).toContain('尚未确认这次行动的最终结果');
    fireEvent.click(await screen.findByRole('button', { name: '继续确认结果' }));
    await waitFor(() => expect(mocks.submit).toHaveBeenCalledTimes(2));

    expect(mocks.submit.mock.calls[1][0].idempotencyKey)
      .toBe(mocks.submit.mock.calls[0][0].idempotencyKey);
  });

  it('unlocks the form after a terminal rejection', async () => {
    mocks.submit.mockRejectedValue({ outcomeUnknown: false });
    render(<WorldDashboard novelId="novel" view={view} />);

    const intent = screen.getByLabelText('你的意图');
    fireEvent.change(intent, { target: { value: '违反规则的行动' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));

    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('请求已被明确拒绝'));
    expect(screen.queryByRole('button', { name: '继续确认结果' })).toBeNull();
    expect(intent.hasAttribute('disabled')).toBe(false);
  });

  it('renders advanced attributes and the persisted server dice result', () => {
    const advancedView = {
      ...view,
      player: {
        ...view.player,
        rules: { mode: 'advanced', attributes: { qinggong: 12 } },
      },
      session: {
        ...view.session,
        game_rules: {
          attributes: [{ key: 'qinggong', label: '轻功', description: '腾挪身法' }],
          action_rules: [],
        },
      },
      journal: [{
        ...view.journal[0],
        resolution: {
          attribute_key: 'qinggong', attribute_label: '轻功', score: 12,
          modifier: 1, roll: 14, total: 15, difficulty_class: 13, succeeded: true,
        },
      }],
    } as unknown as OpenWorldView;

    render(<WorldDashboard novelId="novel" view={advancedView} />);

    expect(screen.getByText('小说属性')).toBeTruthy();
    expect(screen.getByText('轻功检定：D20 14 + 1 = 15 / 难度 13 · 成功')).toBeTruthy();
  });
});
