import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { OpenWorldView } from '@/shared/types';
import { WorldDashboard } from './WorldDashboard';

const mocks = vi.hoisted(() => ({
  submit: vi.fn(),
}));

vi.mock('@/entities/narrative', () => ({
  useSubmitWorldTurn: () => ({ mutateAsync: mocks.submit, isPending: false }),
  isWorldTurnOutcomeUnknown: (error: { outcomeUnknown?: boolean }) => error.outcomeUnknown === true,
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
    state: {
      choices: [{
        node_id: 'choice-node', chapter: 1, choice_index: 0,
        choice: '先去旧城门寻找守门人', consequence: '云舟在旧城门发现了一枚徽记。',
        timestamp: '2026-08-12T23:59:59Z',
      }],
      world_events: [],
      threads: { siege: { status: 'open', description: '围城', origin: 'canon' } },
    },
  },
  journal: [{
    turn_id: 'turn', turn_number: 1,
    memory_projection_status: 'saved',
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
    window.sessionStorage.clear();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('keeps canon provenance distinct and retries a failed turn with the same key', async () => {
    mocks.submit.mockRejectedValue({ outcomeUnknown: true, message: 'offline' });
    const page = render(<WorldDashboard novelId="novel" view={view} />);

    expect(screen.getAllByText(/原著主线/).length).toBeGreaterThan(0);
    expect(screen.getByText(/来源章节 2/)).toBeTruthy();
    // The journey keeps the committed branch prefix before living-world turns
    // and distinguishes reader decisions from generated prose projections.
    expect(screen.getByRole('heading', { name: '旅程时间线' })).toBeTruthy();
    const branchChoice = screen.getByText('先去旧城门寻找守门人');
    expect(screen.getByText(/原著坐标 · 第 1 章/)).toBeTruthy();
    expect(screen.getAllByText(/读者选择/).length).toBeGreaterThan(0);
    expect(screen.getByText(/云舟在旧城门发现了一枚徽记。/)).toBeTruthy();
    expect(screen.getByText(/回合 1/)).toBeTruthy();
    expect(screen.getByText(/读者行动/)).toBeTruthy();
    expect(screen.getByText(/调查线索：探查城门/)).toBeTruthy();
    expect(screen.getAllByText(/生成投影/)).toHaveLength(2);
    expect(screen.getByText(/云舟发现守军换防。/)).toBeTruthy();
    expect(screen.getByText(/2026-08-13T00:00:01Z/)).toBeTruthy();
    expect(branchChoice.compareDocumentPosition(screen.getByText(/回合 1/))
      & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);

    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '前往城门' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));
    await waitFor(() => expect(mocks.submit).toHaveBeenCalledTimes(1));
    expect(mocks.submit.mock.calls[0][0].expectedTurnNumber).toBe(1);
    expect(screen.queryByRole('button', { name: '放弃此请求' })).toBeNull();
    expect(screen.getByRole('alert').textContent).toContain('尚未确认这次行动的最终结果');
    page.rerender(
      <WorldDashboard
        novelId="novel"
        view={{ ...view, session: { ...view.session, turn_number: 2 } }}
      />,
    );
    fireEvent.click(await screen.findByRole('button', { name: '继续确认结果' }));
    await waitFor(() => expect(mocks.submit).toHaveBeenCalledTimes(2));

    expect(mocks.submit.mock.calls[1][0].idempotencyKey)
      .toBe(mocks.submit.mock.calls[0][0].idempotencyKey);
    expect(mocks.submit.mock.calls[1][0].expectedTurnNumber).toBe(1);
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
    expect(window.sessionStorage.length).toBe(0);
  });

  it('clears the stored key after a terminal POST without requiring a journal entry', async () => {
    mocks.submit.mockResolvedValue({ memory_projection_status: 'saved' });
    render(<WorldDashboard novelId="novel" view={{ ...view, journal: [] }} />);

    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '穿过城门' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));

    await waitFor(() => expect(mocks.submit).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(window.sessionStorage.length).toBe(0));
    const intent = screen.getByLabelText('你的意图');
    expect(intent.hasAttribute('disabled')).toBe(false);
    fireEvent.change(intent, { target: { value: '继续前进' } });
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(false);
    expect(screen.queryByRole('button', { name: '继续确认结果' })).toBeNull();
  });

  it('keeps the form and exact key locked until a terminal request finishes refreshing', async () => {
    let finish!: (value: unknown) => void;
    mocks.submit.mockImplementation(() => new Promise(resolve => {
      finish = resolve;
    }));
    render(<WorldDashboard novelId="novel" view={{ ...view, journal: [] }} />);

    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '穿过城门' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));
    await waitFor(() => expect(mocks.submit).toHaveBeenCalledTimes(1));

    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(true);
    expect(window.sessionStorage.length).toBe(1);
    const request = mocks.submit.mock.calls[0][0];

    finish({ memory_projection_status: 'saved' });
    await waitFor(() => expect(window.sessionStorage.length).toBe(0));
    const intent = screen.getByLabelText('你的意图');
    expect(intent.hasAttribute('disabled')).toBe(false);
    fireEvent.change(intent, { target: { value: '继续前进' } });
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(false);
    expect(mocks.submit.mock.calls[0][0]).toEqual(request);
  });

  it('keeps a committed pending projection locked and unlocks only after terminal status', async () => {
    mocks.submit.mockRejectedValue({ outcomeUnknown: true, message: 'refresh failed' });
    const page = render(<WorldDashboard novelId="novel" view={view} />);
    const intent = screen.getByLabelText('你的意图');
    fireEvent.change(intent, { target: { value: '前往城门' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));

    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());
    const idempotencyKey = mocks.submit.mock.calls[0][0].idempotencyKey;
    page.rerender(
      <WorldDashboard
        novelId="novel"
        view={{
          ...view,
          journal: [
            ...view.journal,
            {
              ...view.journal[0],
              turn_id: idempotencyKey,
              turn_number: 2,
              memory_projection_status: 'pending',
            },
          ],
        }}
      />,
    );

    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(true);
    expect(window.sessionStorage.length).toBe(1);

    page.rerender(
      <WorldDashboard
        novelId="novel"
        view={{
          ...view,
          journal: [
            ...view.journal,
            {
              ...view.journal[0],
              turn_id: idempotencyKey,
              turn_number: 2,
              memory_projection_status: 'saved',
            },
          ],
        }}
      />,
    );

    await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(false);
    expect(window.sessionStorage.length).toBe(0);
  });

  it('refreshes a restored pending projection and stops after its terminal status', async () => {
    vi.useFakeTimers();
    const turnId = 'e3744cac-e557-4d78-9d91-9ba060e81c5f';
    window.sessionStorage.setItem(
      'novelworld:pending-world-turn:user:novel',
      JSON.stringify({
        action: { kind: 'travel', target_id: 'gate', intent: '穿过旧城门' },
        idempotencyKey: turnId,
        expectedTurnNumber: 1,
      }),
    );
    const refresh = vi.fn();
    const page = render(
      <WorldDashboard novelId="novel" view={view} onRefresh={refresh} />,
    );

    await act(() => vi.advanceTimersByTimeAsync(10_000));
    expect(refresh).toHaveBeenCalledOnce();

    page.rerender(
      <WorldDashboard
        novelId="novel"
        view={{
          ...view,
          journal: [{
            ...view.journal[0],
            turn_id: turnId,
            turn_number: 2,
            memory_projection_status: 'skipped',
          }],
        }}
        onRefresh={refresh}
      />,
    );
    await act(() => vi.advanceTimersByTimeAsync(30_000));

    expect(refresh).toHaveBeenCalledOnce();
    expect(window.sessionStorage.length).toBe(0);
  });

  it('reconstructs the server pending turn when session storage is unavailable', async () => {
    const turnId = 'e3744cac-e557-4d78-9d91-9ba060e81c5f';
    const pendingEntry = {
      ...view.journal[0],
      turn_id: turnId,
      turn_number: 2,
      memory_projection_status: 'pending' as const,
      action: { kind: 'travel' as const, target_id: 'gate', intent: '穿过旧城门' },
    };
    const getItem = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('storage blocked');
    });
    const setItem = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('storage blocked');
    });
    mocks.submit.mockResolvedValue({ memory_projection_status: 'saved' });
    try {
      render(
        <WorldDashboard novelId="novel" view={{ ...view, journal: [pendingEntry] }} />,
      );

      expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(true);
      fireEvent.click(screen.getByRole('button', { name: '继续确认结果' }));
      await waitFor(() => expect(mocks.submit).toHaveBeenCalledOnce());
      expect(mocks.submit.mock.calls[0][0]).toEqual({
        action: pendingEntry.action,
        idempotencyKey: turnId,
        expectedTurnNumber: 1,
      });
    } finally {
      getItem.mockRestore();
      setItem.mockRestore();
    }
  });

  it('reconstructs the same pending request after tab storage is lost', async () => {
    mocks.submit.mockRejectedValue({ outcomeUnknown: true, message: 'connection lost' });
    const page = render(<WorldDashboard novelId="novel" view={view} />);
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '沿城墙寻找暗门' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));
    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());
    const originalRequest = mocks.submit.mock.calls[0][0];
    page.unmount();
    window.sessionStorage.clear();
    mocks.submit.mockClear();

    render(
      <WorldDashboard
        novelId="novel"
        view={{
          ...view,
          journal: [{
            ...view.journal[0],
            turn_id: originalRequest.idempotencyKey,
            turn_number: originalRequest.expectedTurnNumber + 1,
            memory_projection_status: 'pending',
            action: originalRequest.action,
          }],
        }}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: '继续确认结果' }));

    await waitFor(() => expect(mocks.submit).toHaveBeenCalledOnce());
    expect(mocks.submit.mock.calls[0][0]).toEqual(originalRequest);
  });

  it('lets the committed pending journal replace a different stale tab request', async () => {
    const storedTurnId = '8f84d71c-7674-4b66-b92d-c419ec541b6e';
    const journalTurnId = 'e3744cac-e557-4d78-9d91-9ba060e81c5f';
    const journalAction = { kind: 'travel' as const, target_id: 'gate', intent: '继续已提交行动' };
    window.sessionStorage.setItem(
      'novelworld:pending-world-turn:user:novel',
      JSON.stringify({
        action: { kind: 'investigate', target_id: 'siege', intent: '过时的新行动' },
        idempotencyKey: storedTurnId,
        expectedTurnNumber: 1,
      }),
    );
    mocks.submit.mockResolvedValue({ memory_projection_status: 'saved' });
    render(
      <WorldDashboard
        novelId="novel"
        view={{
          ...view,
          journal: [{
            ...view.journal[0],
            turn_id: journalTurnId,
            turn_number: 2,
            memory_projection_status: 'pending',
            action: journalAction,
          }],
        }}
      />,
    );

    await waitFor(() => expect(JSON.parse(
      window.sessionStorage.getItem('novelworld:pending-world-turn:user:novel') ?? '{}',
    ).idempotencyKey).toBe(journalTurnId));
    fireEvent.click(screen.getByRole('button', { name: '继续确认结果' }));

    await waitFor(() => expect(mocks.submit).toHaveBeenCalledOnce());
    expect(mocks.submit.mock.calls[0][0]).toEqual({
      action: journalAction,
      idempotencyKey: journalTurnId,
      expectedTurnNumber: 1,
    });
  });

  it('does not reconstruct a terminal journal turn', () => {
    render(
      <WorldDashboard
        novelId="novel"
        view={{
          ...view,
          journal: [{
            ...view.journal[0],
            turn_id: '55487c47-9f16-4794-8045-e953c34d36eb',
            memory_projection_status: 'saved',
          }],
        }}
      />,
    );

    expect(screen.queryByRole('button', { name: '继续确认结果' })).toBeNull();
    expect(screen.getByLabelText('你的意图').hasAttribute('disabled')).toBe(false);
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '继续前进' } });
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(false);
  });

  it('restores an ambiguous request after a real unmount with the same action and key', async () => {
    mocks.submit.mockRejectedValue({ outcomeUnknown: true, message: 'connection lost' });
    const page = render(<WorldDashboard novelId="novel" view={view} />);
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '沿城墙寻找暗门' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));

    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());
    const originalRequest = mocks.submit.mock.calls[0][0];
    expect(window.sessionStorage.length).toBe(1);
    page.unmount();

    mocks.submit.mockClear();
    const restoredPage = render(
      <WorldDashboard
        novelId="novel"
        view={{ ...view, session: { ...view.session, turn_number: 2 } }}
      />,
    );
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '另一次行动' } });
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(true);
    expect(screen.getByRole('alert').textContent).toContain('尚未确认这次行动的最终结果');
    fireEvent.click(screen.getByRole('button', { name: '继续确认结果' }));

    await waitFor(() => expect(mocks.submit).toHaveBeenCalledTimes(1));
    expect(mocks.submit.mock.calls[0][0]).toEqual(originalRequest);
    expect(mocks.submit.mock.calls[0][0].expectedTurnNumber).toBe(1);
    restoredPage.unmount();
  });

  it('clears a stale revision and requires a new action from the refreshed turn', async () => {
    mocks.submit
      .mockRejectedValueOnce({ outcomeUnknown: false, message: 'stale revision' })
      .mockResolvedValueOnce({ memory_projection_status: 'saved' });
    const page = render(<WorldDashboard novelId="novel" view={view} />);

    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '旧世界行动' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('请求已被明确拒绝'));
    expect(window.sessionStorage.length).toBe(0);
    expect(mocks.submit.mock.calls[0][0].expectedTurnNumber).toBe(1);

    page.rerender(
      <WorldDashboard
        novelId="novel"
        view={{ ...view, session: { ...view.session, turn_number: 2 } }}
      />,
    );
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '刷新后的行动' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));

    await waitFor(() => expect(mocks.submit).toHaveBeenCalledTimes(2));
    expect(mocks.submit.mock.calls[1][0].expectedTurnNumber).toBe(2);
    expect(mocks.submit.mock.calls[1][0].idempotencyKey)
      .not.toBe(mocks.submit.mock.calls[0][0].idempotencyKey);
  });

  it('does not retry an ambiguous request while timeline mutations are locked', async () => {
    mocks.submit.mockRejectedValue({ outcomeUnknown: true, message: 'connection lost' });
    const page = render(<WorldDashboard novelId="novel" view={view} />);
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '沿城墙寻找暗门' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));
    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());

    mocks.submit.mockClear();
    page.rerender(<WorldDashboard novelId="novel" view={view} actionsDisabled />);
    const retry = screen.getByRole('button', { name: '继续确认结果' });
    expect(retry.hasAttribute('disabled')).toBe(true);
    fireEvent.click(retry);

    expect(mocks.submit).not.toHaveBeenCalled();
  });

  it('isolates restored requests by user and novel and removes invalid storage', async () => {
    mocks.submit.mockRejectedValue({ outcomeUnknown: true, message: 'connection lost' });
    const page = render(<WorldDashboard novelId="novel" view={view} />);
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '留在城门观察' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));
    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());
    page.unmount();

    const otherNovel = render(<WorldDashboard novelId="other-novel" view={view} />);
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '另一本小说的行动' } });
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(false);
    otherNovel.unmount();

    const otherUser = render(
      <WorldDashboard
        novelId="novel"
        view={{ ...view, player: { ...view.player, user_id: 'other-user' } }}
      />,
    );
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '另一位用户的行动' } });
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(false);
    otherUser.unmount();

    window.sessionStorage.setItem('novelworld:pending-world-turn:user:broken', '{bad json');
    const broken = render(<WorldDashboard novelId="broken" view={view} />);
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '损坏数据后的行动' } });
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(false);
    expect(window.sessionStorage.getItem('novelworld:pending-world-turn:user:broken')).toBeNull();
    broken.unmount();

    window.sessionStorage.setItem(
      'novelworld:pending-world-turn:user:oversized',
      JSON.stringify({ idempotencyKey: crypto.randomUUID(), action: { kind: 'travel', target_id: 'gate', intent: 'A'.repeat(5_000) } }),
    );
    render(<WorldDashboard novelId="oversized" view={view} />);
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '越界数据后的行动' } });
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(false);
    expect(window.sessionStorage.getItem('novelworld:pending-world-turn:user:oversized')).toBeNull();
  });

  it('preserves line breaks and safely wraps long legacy timeline text', () => {
    const token = 'A'.repeat(500);
    const choice = `第一行选择\n${token}`;
    const consequence = `第一行选择投影\n${token}`;
    const action = `第一行行动\n${token}`;
    const projection = `第一行行动投影\n${token}`;
    const { container } = render(
      <WorldDashboard
        novelId="novel"
        view={{
          ...view,
          world_state: {
            ...view.world_state,
            state: {
              ...view.world_state.state,
              choices: [{ chapter: 1, choice, consequence }],
            },
          },
          journal: [{
            ...view.journal[0],
            action: { ...view.journal[0].action, intent: action },
            transition: { ...view.journal[0].transition, rendered_narrative: projection },
          }],
        }}
      />,
    );

    const timelineText = Array.from(container.querySelectorAll('.whitespace-pre-wrap'));
    expect(timelineText).toHaveLength(4);
    expect(timelineText.every(element => (
      element.classList.contains('[overflow-wrap:anywhere]')
    ))).toBe(true);
    expect(timelineText.map(element => element.textContent)).toEqual([
      choice,
      consequence,
      `调查线索：${action}`,
      projection,
    ]);
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
