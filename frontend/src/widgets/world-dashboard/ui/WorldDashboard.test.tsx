import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { WorldTurnConfirmation } from '@/entities/narrative';
import type { OpenWorldView } from '@/shared/types';
import { WorldDashboard } from './WorldDashboard';

const mocks = vi.hoisted(() => ({
  submit: vi.fn(),
  confirmation: {
    data: undefined as { confirmation: WorldTurnConfirmation; refreshedWorld?: OpenWorldView } | undefined,
    isFetching: false, isError: false, refetch: vi.fn(),
  },
}));

vi.mock('@/entities/narrative', () => ({
  useSubmitWorldTurn: () => ({ mutateAsync: mocks.submit, isPending: false }),
  useWorldTurnConfirmation: () => mocks.confirmation,
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

function chooseTravel() {
  const action = screen.getByLabelText('行动') as HTMLSelectElement;
  if (action.disabled) return;
  fireEvent.change(action, { target: { value: 'travel' } });
  fireEvent.change(screen.getByLabelText('目标'), { target: { value: 'gate' } });
}

describe('WorldDashboard', () => {
  beforeEach(() => {
    mocks.submit.mockReset();
    mocks.confirmation.data = undefined;
    mocks.confirmation.isFetching = false;
    mocks.confirmation.isError = false;
    mocks.confirmation.refetch.mockReset();
    window.sessionStorage.clear();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('permits only the exact authoritative turn retry in recovery-only mode', async () => {
    mocks.submit.mockResolvedValue(undefined);
    const recovery = { turn_id: 'ed3f5292-9492-4537-afcf-468657f1d8c7', action: { kind: 'travel' as const, target_id: 'gate', intent: '恢复原旅程' }, expected_turn_number: 1, expected_source_chapter: 2 };
    render(<WorldDashboard novelId="novel" view={{ ...view, recoverable_turn: recovery }} recoveryOnly />);
    expect((screen.getByLabelText('行动') as HTMLSelectElement).disabled).toBe(true);
    expect((screen.getByRole('button', { name: '执行行动' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: '恢复原行动' }));
    await waitFor(() => expect(mocks.submit).toHaveBeenCalledOnce());
    expect(mocks.submit).toHaveBeenCalledWith({ action: recovery.action, idempotencyKey: recovery.turn_id, expectedTurnNumber: 1, expectedSourceChapter: 2 });
    expect((screen.getByLabelText('行动') as HTMLSelectElement).disabled).toBe(true);
  });

  it('reads confirmation without retrying the action, including under a mutation lock', () => {
    const request = { action: { kind: 'travel', target_id: 'gate', intent: '前往城门' }, idempotencyKey: '80470e95-87cf-4c50-a05c-f7743c43c079', expectedTurnNumber: 1 };
    window.sessionStorage.setItem('novelworld:pending-world-turn:user:novel', JSON.stringify(request));
    render(<WorldDashboard novelId="novel" view={view} actionsDisabled />);
    fireEvent.click(screen.getByRole('button', { name: '继续确认结果' }));
    expect(mocks.confirmation.refetch).toHaveBeenCalledOnce();
    expect(mocks.submit).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: '恢复原行动' }).hasAttribute('disabled')).toBe(true);
    expect(JSON.parse(window.sessionStorage.getItem('novelworld:pending-world-turn:user:novel') ?? '{}')).toEqual(request);
  });

  it('automatically releases a durably failed stored action after its fresh world is visible', async () => {
    const turnId = '80470e95-87cf-4c50-a05c-f7743c43c079';
    window.sessionStorage.setItem('novelworld:pending-world-turn:user:novel', JSON.stringify({ action: { kind: 'travel', target_id: 'gate', intent: '前往城门' }, idempotencyKey: turnId, expectedTurnNumber: 1 }));
    mocks.confirmation.data = { confirmation: { turn_id: turnId, status: 'failed' }, refreshedWorld: view };
    render(<WorldDashboard novelId="novel" view={view} />);
    await waitFor(() => expect(window.sessionStorage.getItem('novelworld:pending-world-turn:user:novel')).toBeNull());
    expect(screen.queryByRole('button', { name: '继续确认结果' })).toBeNull();
    expect(screen.getByText(/failed before it changed your world/)).toBeTruthy();
    expect(mocks.submit).not.toHaveBeenCalled();
  });

  it.each(['isFetching', 'isError'] as const)('does not unlock from cached failure during %s', state => {
    const turnId = '80470e95-87cf-4c50-a05c-f7743c43c079';
    window.sessionStorage.setItem('novelworld:pending-world-turn:user:novel', JSON.stringify({ action: { kind: 'travel', target_id: 'gate', intent: '前往城门' }, idempotencyKey: turnId, expectedTurnNumber: 1 }));
    mocks.confirmation.data = { confirmation: { turn_id: turnId, status: 'failed' }, refreshedWorld: view };
    mocks.confirmation[state] = true;
    render(<WorldDashboard novelId="novel" view={view} />);
    expect(window.sessionStorage.getItem('novelworld:pending-world-turn:user:novel')).not.toBeNull();
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(true);
    expect(mocks.submit).not.toHaveBeenCalled();
  });

  it('keeps a confirmed failure locked when the current world cannot refresh', () => {
    const turnId = '80470e95-87cf-4c50-a05c-f7743c43c079';
    window.sessionStorage.setItem('novelworld:pending-world-turn:user:novel', JSON.stringify({ action: { kind: 'travel', target_id: 'gate', intent: '前往城门' }, idempotencyKey: turnId, expectedTurnNumber: 1 }));
    mocks.confirmation.data = { confirmation: { turn_id: turnId, status: 'failed' } };
    render(<WorldDashboard novelId="novel" view={view} />);
    expect(screen.getByRole('button', { name: '继续确认结果' })).toBeTruthy();
    expect(screen.queryByRole('button', { name: '恢复原行动' })).toBeNull();
    expect(window.sessionStorage.getItem('novelworld:pending-world-turn:user:novel')).not.toBeNull();
    expect(mocks.submit).not.toHaveBeenCalled();
  });

  it('waits for a completed confirmation to advance the visible world before unlocking', async () => {
    const turnId = '80470e95-87cf-4c50-a05c-f7743c43c079';
    const request = { action: { kind: 'travel', target_id: 'gate', intent: '前往城门' }, idempotencyKey: turnId, expectedTurnNumber: 1 };
    window.sessionStorage.setItem('novelworld:pending-world-turn:user:novel', JSON.stringify(request));
    const fresh = { ...view, session: { ...view.session, turn_number: 2 }, world_state: { ...view.world_state, updated_at: '2026-08-13T00:00:02Z' } };
    mocks.confirmation.data = { confirmation: { turn_id: turnId, status: 'completed', memory_projection_status: 'saved' }, refreshedWorld: fresh };
    const page = render(<WorldDashboard novelId="novel" view={view} />);
    expect(window.sessionStorage.getItem('novelworld:pending-world-turn:user:novel')).not.toBeNull();
    page.rerender(<WorldDashboard novelId="novel" view={fresh} />);
    await waitFor(() => expect(window.sessionStorage.getItem('novelworld:pending-world-turn:user:novel')).toBeNull());
    expect(mocks.submit).not.toHaveBeenCalled();
  });

  it('cannot clear a replacement server turn with a late failed confirmation', () => {
    const oldId = '80470e95-87cf-4c50-a05c-f7743c43c079';
    const active = { turn_id: 'ed3f5292-9492-4537-afcf-468657f1d8c7', action: { kind: 'travel' as const, target_id: 'gate', intent: '恢复原旅程' }, expected_turn_number: 1 };
    const fresh = { ...view, recoverable_turn: active };
    mocks.confirmation.data = { confirmation: { turn_id: oldId, status: 'failed' }, refreshedWorld: view };
    render(<WorldDashboard novelId="novel" view={fresh} />);
    expect(JSON.parse(window.sessionStorage.getItem('novelworld:pending-world-turn:user:novel') ?? '{}').idempotencyKey).toBe(active.turn_id);
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(true);
    expect(mocks.submit).not.toHaveBeenCalled();
  });

  it('links the current narrative to the next action without submitting automatically', () => {
    render(<WorldDashboard novelId="novel" view={view} />);
    expect(screen.getByRole('link', { name: '去选择行动' }).getAttribute('href')).toBe('#world-action-form');
    expect(screen.getByText(/确认意图后点击“执行行动”，故事会继续推进/)).toBeTruthy();
    expect(mocks.submit).not.toHaveBeenCalled();
  });

  it.each(['user_id', 'novel_id', 'id'] as const)('clears action drafts when player %s changes', field => {
    const page = render(<WorldDashboard novelId="novel" view={view} />);
    chooseTravel();
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '留在旧世界的私有草稿' } });

    const next = { ...view, player: { ...view.player, [field]: `other-${field}` } };
    page.rerender(<WorldDashboard novelId={next.player.novel_id} view={next} />);

    expect((screen.getByLabelText('你的意图') as HTMLTextAreaElement).value).toBe('');
    expect((screen.getByLabelText('行动') as HTMLSelectElement).value).toBe('pursue_goal');
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(true);
    expect(mocks.submit).not.toHaveBeenCalled();
  });

  it('explains a stale view lock at the action form and offers a refresh', () => {
    const refresh = vi.fn();
    render(<WorldDashboard novelId="novel" view={view} actionsDisabled
      actionsDisabledReason="世界加载失败，请重试。" onRefresh={refresh} />);
    const alert = screen.getByRole('alert');
    expect(alert.textContent).toContain('世界加载失败');
    expect(alert.parentElement?.querySelector('form')).toBeTruthy();
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: '重试' }));
    expect(refresh).toHaveBeenCalledOnce();
    expect(mocks.submit).not.toHaveBeenCalled();
  });

  it('shows an unspecified location for a player created before any place is available', () => {
    render(<WorldDashboard novelId="novel" view={{
      ...view,
      player: { ...view.player, location_id: null },
      session: { ...view.session, entry_context: { ...view.session.entry_context, locations: [] } },
    }} />);
    expect(screen.getByText(/地点未确认/)).toBeTruthy();
  });

  it('shows independently recorded local character actions without showing distant actors', () => {
    render(<WorldDashboard novelId="novel" view={{
      ...view,
      session: {
        ...view.session,
        entry_context: {
          ...view.session.entry_context,
          characters: [
            { id: 'near', name: '守门人' },
            { id: 'away', name: '信使' },
          ],
        },
      },
      journal: [{
        ...view.journal[0],
        transition: {
          ...view.journal[0].transition,
          events: [
            { summary: '守门人关上侧门。', actor_character_ids: ['near'], location_id: 'gate' },
            { summary: '信使已在驿站出发。', actor_character_ids: ['away'], location_id: 'station' },
          ],
        },
      }],
    }} />);

    expect(screen.getByText('守门人关上侧门。')).toBeTruthy();
    expect(screen.queryByText('信使已在驿站出发。')).toBeNull();
    expect(screen.getByText('守门人')).toBeTruthy();
  });

  it('keeps canon provenance distinct and retries a failed turn with the same key', async () => {
    mocks.submit.mockRejectedValue(Object.assign(
      new Error('已提交行动的记忆尚未确认'), { outcomeUnknown: true },
    ));
    const page = render(<WorldDashboard novelId="novel" view={view} />);

    expect(screen.getAllByText(/原著主线/).length).toBeGreaterThan(0);
    expect(screen.getByText('原著抽取')).toBeTruthy();
    expect(screen.getByText('事件由模型从原著中抽取，可能存在遗漏或误读，请结合来源章节核对。')).toBeTruthy();
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
    expect(screen.getAllByText(/生成投影/)).toHaveLength(1);
    expect(screen.getAllByText(/云舟发现守军换防。/)).toHaveLength(1);
    expect(screen.getByText(/2026-08-13T00:00:01Z/)).toBeTruthy();
    expect(branchChoice.compareDocumentPosition(screen.getByText(/回合 1/))
      & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);

    chooseTravel();
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '前往城门' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));
    await waitFor(() => expect(mocks.submit).toHaveBeenCalledTimes(1));
    expect(mocks.submit.mock.calls[0][0].expectedTurnNumber).toBe(1);
    expect(screen.queryByRole('button', { name: '放弃此请求' })).toBeNull();
    expect(screen.getByRole('alert').textContent).toContain('尚未确认这次行动的最终结果');
    expect(screen.getByRole('alert').textContent).toContain('已提交行动的记忆尚未确认');
    page.rerender(
      <WorldDashboard
        novelId="novel"
        view={{ ...view, session: { ...view.session, turn_number: 2 } }}
      />,
    );
    fireEvent.click(await screen.findByRole('button', { name: '恢复原行动' }));
    await waitFor(() => expect(mocks.submit).toHaveBeenCalledTimes(2));

    expect(mocks.submit.mock.calls[1][0].idempotencyKey)
      .toBe(mocks.submit.mock.calls[0][0].idempotencyKey);
    expect(mocks.submit.mock.calls[1][0].expectedTurnNumber).toBe(1);
  });

  it('discloses extraction provenance while preserving event status and source chapters', () => {
    const withPlayerAffectedEvent = {
      ...view,
      session: {
        ...view.session,
        canonical_events: [
          ...view.session.canonical_events,
          { id: 'scheduled-event', sequence: 2, summary: '尚未发生的事件', character_ids: [], location_ids: ['gate'], faction_ids: [], death_character_ids: [], source_chapters: [5], status: 'scheduled' as const, reason: '尚未触发' },
          { id: 'assisted-event', sequence: 3, summary: '玩家影响的事件', character_ids: [], location_ids: ['gate'], faction_ids: [], death_character_ids: [], source_chapters: [3, 4], status: 'assisted' as const, reason: '读者守住城门', advanced_at_world_time: 2 },
        ],
      },
    } satisfies OpenWorldView;
    render(<WorldDashboard novelId="novel" view={withPlayerAffectedEvent} />);

    const rows = screen.getAllByRole('listitem').map(row => row.textContent ?? '');
    expect(rows.some(row => row.includes('围城开始') && row.includes('原著抽取') && row.includes('被延迟') && row.includes('来源章节 2') && row.includes('城门未开'))).toBe(true);
    expect(rows.some(row => row.includes('尚未发生的事件') && row.includes('原著抽取') && row.includes('等待发生') && row.includes('来源章节 5') && row.includes('尚未触发'))).toBe(true);
    expect(rows.some(row => row.includes('玩家影响的事件') && row.includes('原著抽取') && row.includes('玩家协助') && row.includes('世界时间 2') && row.includes('来源章节 3、4') && row.includes('读者守住城门'))).toBe(true);
    expect(screen.getByText('事件由模型从原著中抽取，可能存在遗漏或误读，请结合来源章节核对。')).toBeTruthy();
  });

  it('labels only explicit thread provenance as canon or player', () => {
    const unmarked = {
      ...view,
      world_state: {
        ...view.world_state,
        state: {
          ...view.world_state.state,
          threads: {
            canon: { status: 'open', description: '原著线', origin: 'canon' },
            player: { status: 'open', description: '玩家线', origin: 'player' },
            absent: { status: 'open', description: '未标记线' },
            unexpected: { status: 'open', description: '异常线', origin: 'model' as never },
          },
        },
      },
    } satisfies OpenWorldView;
    render(<WorldDashboard novelId="novel" view={unmarked} />);

    const rows = screen.getAllByRole('listitem').map(row => row.textContent);
    expect(rows).toContain('原著线 · 原著主线');
    expect(rows).toContain('玩家线 · 玩家创造');
    expect(rows).toContain('未标记线 · 来源未确认');
    expect(rows).toContain('异常线 · 来源未确认');
  });

  it('unlocks the form after a terminal rejection', async () => {
    mocks.submit.mockRejectedValue({ outcomeUnknown: false });
    render(<WorldDashboard novelId="novel" view={view} />);

    chooseTravel();
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

    chooseTravel();
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

    chooseTravel();
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
    chooseTravel();
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
      expect(screen.getByRole('alert').textContent).toContain('第 2 回合的经过已保存，但角色记忆尚未同步完成');
      fireEvent.click(screen.getByRole('button', { name: '恢复原行动' }));
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

  it('recovers an active server turn after reload when session storage is unavailable', async () => {
    const turnId = 'e3744cac-e557-4d78-9d91-9ba060e81c5f';
    const action = { kind: 'travel' as const, target_id: 'gate', intent: '穿过旧城门' };
    const getItem = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('storage blocked');
    });
    const setItem = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('storage blocked');
    });
    mocks.submit.mockResolvedValue({ memory_projection_status: 'saved' });
    try {
      render(
        <WorldDashboard
          novelId="novel"
          view={{
            ...view,
            journal: [],
            recoverable_turn: {
              turn_id: turnId,
              action,
              expected_turn_number: 1,
            },
          }}
        />,
      );

      expect(mocks.submit).not.toHaveBeenCalled();
      expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(true);
      expect(screen.getByRole('alert').textContent).toContain('上一行动尚未完成');
      fireEvent.click(screen.getByRole('button', { name: '恢复原行动' }));
      await waitFor(() => expect(mocks.submit).toHaveBeenCalledOnce());
      expect(mocks.submit.mock.calls[0][0]).toEqual({
        action,
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
    chooseTravel();
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
            expected_source_chapter: originalRequest.expectedSourceChapter,
            memory_projection_status: 'pending',
            action: originalRequest.action,
          }],
        }}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: '恢复原行动' }));

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
    fireEvent.click(screen.getByRole('button', { name: '恢复原行动' }));

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
    chooseTravel();
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '继续前进' } });
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(false);
  });

  it('restores an ambiguous request after a real unmount with the same action and key', async () => {
    mocks.submit.mockRejectedValue({ outcomeUnknown: true, message: 'connection lost' });
    const page = render(<WorldDashboard novelId="novel" view={view} />);
    chooseTravel();
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
    chooseTravel();
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '另一次行动' } });
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(true);
    expect(screen.getByRole('alert').textContent).toContain('尚未确认这次行动的最终结果');
    fireEvent.click(screen.getByRole('button', { name: '恢复原行动' }));

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

    chooseTravel();
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
    chooseTravel();
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
    chooseTravel();
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '沿城墙寻找暗门' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));
    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());

    mocks.submit.mockClear();
    page.rerender(<WorldDashboard novelId="novel" view={view} actionsDisabled />);
    const retry = screen.getByRole('button', { name: '恢复原行动' });
    expect(retry.hasAttribute('disabled')).toBe(true);
    fireEvent.click(retry);

    expect(mocks.submit).not.toHaveBeenCalled();
  });

  it('isolates restored requests by user and novel and removes invalid storage', async () => {
    mocks.submit.mockRejectedValue({ outcomeUnknown: true, message: 'connection lost' });
    const page = render(<WorldDashboard novelId="novel" view={view} />);
    chooseTravel();
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '留在城门观察' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));
    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());
    page.unmount();

    const otherNovel = render(<WorldDashboard novelId="other-novel" view={view} />);
    chooseTravel();
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '另一本小说的行动' } });
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(false);
    otherNovel.unmount();

    const otherUser = render(
      <WorldDashboard
        novelId="novel"
        view={{ ...view, player: { ...view.player, user_id: 'other-user' } }}
      />,
    );
    chooseTravel();
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '另一位用户的行动' } });
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(false);
    otherUser.unmount();

    window.sessionStorage.setItem('novelworld:pending-world-turn:user:broken', '{bad json');
    const broken = render(<WorldDashboard novelId="broken" view={view} />);
    chooseTravel();
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '损坏数据后的行动' } });
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(false);
    expect(window.sessionStorage.getItem('novelworld:pending-world-turn:user:broken')).toBeNull();
    broken.unmount();

    window.sessionStorage.setItem(
      'novelworld:pending-world-turn:user:oversized',
      JSON.stringify({ idempotencyKey: crypto.randomUUID(), action: { kind: 'travel', target_id: 'gate', intent: 'A'.repeat(5_000) } }),
    );
    render(<WorldDashboard novelId="oversized" view={view} />);
    chooseTravel();
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

    const timelineText = Array.from(container.querySelectorAll('[role="log"] .whitespace-pre-wrap'));
    expect(timelineText).toHaveLength(3);
    expect(timelineText.every(element => (
      element.classList.contains('[overflow-wrap:anywhere]')
    ))).toBe(true);
    expect(timelineText.map(element => element.textContent)).toEqual([
      choice,
      consequence,
      `调查线索：${action}`,
    ]);
  });

  it('renders advanced attributes and a persisted legacy server dice result', () => {
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
    expect(screen.getAllByText('轻功检定：D20 14 + 1 = 15 / 难度 13 · 成功')).toHaveLength(2);
  });

  it.each([
    { decision: 'template_fallback', succeeded: false, explanation: '本次检定失败', summary: 'D20 6 + 1 = 7 / 难度 12 · 失败' },
    { decision: 'standard_check', succeeded: false, explanation: '本次检定失败', summary: 'D20 6 + 1 = 7 / 难度 12 · 失败' },
    { decision: 'standard_check', succeeded: true, explanation: '本回合已完成', summary: 'D20 14 + 1 = 15 / 难度 12 · 成功' },
    { decision: 'impossible', succeeded: true, explanation: '该行动不可行', summary: '未进行骰子检定' },
    { decision: 'pending', succeeded: true, explanation: '行动判断尚未完成', summary: '判断未完成' },
    { decision: 'automatic_success', succeeded: false, explanation: '本回合已完成', summary: '无需检定 · 行动成功' },
  ] as const)('explains the latest $decision result beside its narrative', ({ decision, succeeded, explanation, summary }) => {
    render(<WorldDashboard novelId="novel" view={{
      ...view,
      journal: [{
        ...view.journal[0],
        resolution: {
          schema_version: 1, canon_model_version: 1, template_prompt_version: 'basic-rules-v1',
          attribute_key: 'qinggong', attribute_label: '轻功', score: 12, modifier: 1,
          roll: succeeded ? 14 : 6, total: succeeded ? 15 : 7, difficulty_class: 12, succeeded,
          adjudication: { schema_version: 1, template_difficulty_class: 12, decision },
        },
      }],
    }} />);
    const result = screen.getByRole('status', { name: '本回合行动结果' });
    expect(result.textContent).toContain(explanation);
    expect(result.textContent).toContain(summary);
    expect(result.parentElement?.querySelector('#latest-world-narrative')).toBeTruthy();
    expect(screen.getByText(/世界入场坐标 · 原著第 1 章/)).toBeTruthy();
    expect(screen.getByLabelText('行动').hasAttribute('disabled')).toBe(false);
    expect(mocks.submit).not.toHaveBeenCalled();
  });

  it('shows the full latest narrative, world-time tick rule, and event-linked attribute deltas', () => {
    const narrative = '守门人打开城门。'.repeat(35);
    const advancedView = {
      ...view,
      player: {
        ...view.player,
        rules: { mode: 'advanced', attributes: { qinggong: 12, neili: 8 } },
      },
      session: {
        ...view.session,
        game_rules: {
          attributes: [
            { key: 'qinggong', label: '轻功', description: '腾挪身法' },
            { key: 'neili', label: '内力', description: '内息修为' },
          ],
          action_rules: [],
        },
      },
      journal: [{
        ...view.journal[0],
        transition: {
          ...view.journal[0].transition,
          rendered_narrative: narrative,
          events: [{ summary: '守门人打开城门。', actor_character_ids: [], location_id: 'gate' }],
          attribute_changes: [
            { attribute_key: 'qinggong', delta: 2, reason: '借助城墙跃上门楼', event_index: 0 },
            { attribute_key: 'neili', delta: -1, reason: '消耗内息', event_index: 0 },
          ],
        },
      }],
    } as unknown as OpenWorldView;

    render(<WorldDashboard novelId="novel" view={advancedView} />);

    expect(screen.getByText(/世界时间 1 · 每次已提交回合推进 1 步/)).toBeTruthy();
    expect(screen.getAllByText(narrative)).toHaveLength(1);
    expect(screen.queryByText('展开完整经过')).toBeNull();
    const announcedNarrative = screen.getByText(narrative);
    expect(announcedNarrative.getAttribute('role')).toBe('status');
    expect(announcedNarrative.getAttribute('aria-live')).toBe('polite');
    expect(screen.getByRole('link', { name: '查看本回合完整叙事' }).getAttribute('href'))
      .toBe('#latest-world-narrative');
    expect(screen.getByText('+2 · 借助城墙跃上门楼')).toBeTruthy();
    expect(screen.getByText('-1 · 消耗内息').className).toContain('text-[#b3261e]');
    expect(screen.getByText(/守门人打开城门。：轻功 \+2 · 借助城墙跃上门楼/)).toBeTruthy();
    expect(screen.getByText(/内力 -1 · 消耗内息/).className).toContain('text-[#b3261e]');
  });

  it('renders adjudicated no-check, semantic-check, fallback, and pending journal outcomes', () => {
    const cases = [
      { intent: '不可行行动', decision: 'impossible', succeeded: true },
      { intent: '自动成功行动', decision: 'automatic_success', succeeded: false },
      { intent: '低难度行动', decision: 'easy_check', succeeded: true },
      { intent: '模板回退行动', decision: 'template_fallback', succeeded: false },
      { intent: '未完成行动', decision: 'pending', succeeded: true },
    ] as const;
    const adjudicatedView = {
      ...view,
      journal: cases.map(({ intent, decision, succeeded }, index) => ({
        ...view.journal[0],
        turn_id: `turn-${index}`,
        turn_number: index + 1,
        action: { ...view.journal[0].action, intent },
        resolution: {
          attribute_key: 'qinggong', attribute_label: '轻功', score: 12,
          modifier: 1, roll: 14, total: 15, difficulty_class: 13, succeeded,
          adjudication: {
            schema_version: 1,
            template_difficulty_class: 13,
            decision,
          },
        },
      })),
    } as unknown as OpenWorldView;

    render(<WorldDashboard novelId="novel" view={adjudicatedView} />);

    const rowFor = (intent: string) => screen.getByText(new RegExp(intent)).closest('li')!;
    const impossible = rowFor('不可行行动');
    expect(impossible.textContent).toContain('行动不可行 · 未进行骰子检定');
    expect(impossible.textContent).not.toContain('D20');
    const automatic = rowFor('自动成功行动');
    expect(automatic.textContent).toContain('无需检定 · 行动成功');
    expect(automatic.textContent).not.toContain('D20');
    expect(rowFor('低难度行动').textContent).toContain(
      '轻功检定：D20 14 + 1 = 15 / 难度 13 · 成功 · 语义难度：低',
    );
    expect(rowFor('模板回退行动').textContent).toContain(
      '轻功检定：D20 14 + 1 = 15 / 难度 13 · 失败 · 沿用模板检定',
    );
    const pending = rowFor('未完成行动');
    expect(pending.textContent).toContain('判断未完成');
    expect(pending.textContent).not.toContain('行动成功');
    const pendingSummary = Array.from(pending.querySelectorAll('div'))
      .find(element => element.textContent === '判断未完成');
    expect(pendingSummary?.classList.contains('text-[#0d652d]')).toBe(false);
  });
});
