import { beforeEach as beforeLocaleTest } from 'vitest';
import { setLocale } from '@/shared/lib/i18n';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { suggestWorldAction } from '@/entities/narrative';
import type { OpenWorldView } from '@/shared/types';
import { WorldActionForm } from './WorldActionForm';

vi.mock('@/entities/narrative', () => ({ suggestWorldAction: vi.fn() }));

const view = {
  player: { id: 'player', name: '云舟', novel_id: 'novel', location_id: 'gate' },
  session: {
    turn_number: 1,
    entry_context: {
      checkpoint_chapter: 1,
      unlocked_through_chapter: 2,
      locations: [{ id: 'gate', name: '旧城门' }, { id: 'tower', name: '北塔' }],
      characters: [{ id: 'character', name: '守门人' }],
      dead_character_ids: [],
      threads: [{ id: 'siege', name: '围城线' }],
      scheduled_events: [{
        id: 'siege-event', sequence: 1, summary: '围城开始', character_ids: [],
        location_ids: ['gate'], faction_ids: [], death_character_ids: [], source_chapters: [2],
      }],
      character_goals: [{ id: 'canon-goal', character_id: 'character', description: '守住城门', source_chapters: [1] }],
    },
    canonical_events: [{
      id: 'siege-event', sequence: 1, summary: '围城开始', character_ids: [], location_ids: ['gate'],
      faction_ids: [], death_character_ids: [], source_chapters: [2], status: 'delayed', reason: '仍在继续',
    }],
    dead_character_ids: [],
  },
  world_state: { state: { threads: { siege: { status: 'open', description: '围城仍在继续' } } } },
  journal: [{ turn_number: 1, transition: { events: [
    { location_id: 'gate', actor_character_ids: ['character'] },
  ] } }],
} as unknown as OpenWorldView;

function openAdvanced() {
  fireEvent.click(screen.getByText('调整行动方式与目标'));
}

describe('WorldActionForm', () => {
  beforeEach(() => vi.clearAllMocks());

  it('shows three numbered scene suggestions and a free-input entry', () => {
    render(<WorldActionForm view={view} isPending={false} onSubmit={vi.fn()} />);
    const group = screen.getByRole('group', { name: '场景建议' });
    expect(within(group).getAllByRole('button')).toHaveLength(3);
    expect(within(group).getByRole('button', { name: '1. 与守门人交谈' })).toBeTruthy();
    expect(within(group).getByRole('button', { name: '2. 留意围城仍在继续' })).toBeTruthy();
    expect(within(group).getByRole('button', { name: '3. 前往北塔' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '自由输入' })).toBeTruthy();
  });

  it('binds a scene suggestion to its valid target but waits for explicit execution', async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(<WorldActionForm view={view} isPending={false} onSubmit={onSubmit} />);

    fireEvent.click(screen.getByRole('button', { name: '2. 留意围城仍在继续' }));
    expect(screen.getByLabelText('你的意图')).toHaveProperty('value', '看看围城仍在继续的情况，想好下一步。');
    expect(onSubmit).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith({
      kind: 'investigate', target_id: 'siege', intent: '看看围城仍在继续的情况，想好下一步。',
    }));
  });

  it('filters branch-created threads and future or unadmitted events from suggestions and targets', () => {
    const futureEvent = {
      id: 'future-event', sequence: 2, summary: '未来事件', character_ids: [], location_ids: ['gate'],
      faction_ids: [], death_character_ids: [], source_chapters: [3],
    };
    const restricted = {
      ...view,
      session: {
        ...view.session,
        entry_context: {
          ...view.session.entry_context,
          unlocked_through_chapter: 2,
          scheduled_events: [...view.session.entry_context.scheduled_events, futureEvent],
        },
        canonical_events: [
          ...view.session.canonical_events,
          { ...futureEvent, status: 'scheduled', reason: null },
          { ...view.session.canonical_events[0], id: 'unadmitted-event', summary: '未准入事件', source_chapters: [2] },
        ],
      },
      world_state: { state: { threads: {
        siege: view.world_state.state.threads?.siege,
        branch: { status: 'open', description: '分支事件线' },
      } } },
    } as unknown as OpenWorldView;
    render(<WorldActionForm view={restricted} isPending={false} onSubmit={vi.fn()} />);
    expect(screen.getByRole('button', { name: '2. 留意围城仍在继续' })).toBeTruthy();
    expect(screen.queryByRole('button', { name: /分支事件线|未来事件|未准入事件/ })).toBeNull();

    fireEvent.change(screen.getByLabelText('行动'), { target: { value: 'advance_thread' } });
    expect(screen.getByRole('option', { name: '围城仍在继续' })).toBeTruthy();
    expect(screen.queryByRole('option', { name: '分支事件线' })).toBeNull();
    fireEvent.change(screen.getByLabelText('行动'), { target: { value: 'investigate' } });
    expect(screen.queryByRole('option', { name: '主线事件：未来事件' })).toBeNull();
    expect(screen.queryByRole('option', { name: '主线事件：未准入事件' })).toBeNull();
  });

  it('submits free input with no target and no kind-selection step', async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(<WorldActionForm view={view} isPending={false} onSubmit={onSubmit} />);
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '绘制自己的世界地图' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith({
      kind: 'pursue_goal', target_id: null, intent: '绘制自己的世界地图',
    }));
  });

  it('discards an untouched suggestion when its scene changes', () => {
    const { rerender } = render(<WorldActionForm view={view} isPending={false} onSubmit={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: '1. 与守门人交谈' }));
    rerender(<WorldActionForm view={{ ...view, session: { ...view.session, turn_number: 2 } }} isPending={false} onSubmit={vi.fn()} />);
    expect(screen.getByLabelText('你的意图')).toHaveProperty('value', '');
    expect(screen.getByRole('button', { name: '自由输入' }).getAttribute('aria-pressed')).toBe('true');
  });

  it('requires one confirmation before using an authored draft after its scene changes', async () => {
    const onSubmit = vi.fn();
    const { rerender } = render(<WorldActionForm view={view} isPending={false} onSubmit={onSubmit} />);
    fireEvent.click(screen.getByRole('button', { name: '1. 与守门人交谈' }));
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '问守门人城门是否安全' } });

    const changed = {
      ...view,
      session: { ...view.session, turn_number: 2 },
      journal: [{ turn_number: 2, transition: { events: [
        { location_id: 'gate', actor_character_ids: ['character'] },
      ] } }],
    } as unknown as OpenWorldView;
    rerender(<WorldActionForm view={changed} isPending={false} onSubmit={onSubmit} />);

    expect(screen.getByLabelText('你的意图')).toHaveProperty('value', '问守门人城门是否安全');
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: '确认在当前场景继续' }));
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith({
      kind: 'pursue_goal', target_id: null, intent: '问守门人城门是否安全',
    }));
  });

  it('keeps an equivalent refreshed view draft and selection', () => {
    const onSubmit = vi.fn();
    const { rerender } = render(<WorldActionForm view={view} isPending={false} onSubmit={onSubmit} />);
    fireEvent.click(screen.getByRole('button', { name: '3. 前往北塔' }));
    const refreshed = { ...view, world_state: { ...view.world_state } };
    rerender(<WorldActionForm view={refreshed} isPending={false} onSubmit={onSubmit} />);
    expect(screen.getByLabelText('你的意图')).toHaveProperty('value', '前往北塔，看看那里的情况。');
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(false);
  });

  it('preserves a draft edited through its target across source changes until confirmed', async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const page = render(<WorldActionForm view={view} isPending={false} onSubmit={onSubmit} />);
    const suggestion = screen.getByRole('button', { name: /^3\./ });
    fireEvent.click(suggestion);
    openAdvanced();
    fireEvent.change(screen.getByLabelText('目标'), { target: { value: 'gate' } });
    const intent = (screen.getByLabelText('你的意图') as HTMLTextAreaElement).value;

    page.rerender(<WorldActionForm view={{ ...view, session: {
      ...view.session, source_context: { ...view.session.entry_context, unlocked_through_chapter: 3 },
    } }} isPending={false} onSubmit={onSubmit} />);

    expect(screen.getByLabelText('你的意图')).toHaveProperty('value', intent);
    expect(screen.getByLabelText('目标')).toHaveProperty('value', 'gate');
    expect(screen.getByRole('button', { name: /^3\./ }).getAttribute('aria-pressed')).toBe('false');
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(true);
    expect(onSubmit).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: '确认在当前场景继续' }));
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith({ kind: 'travel', target_id: 'gate', intent }));
  });

  it('binds a blank draft to a changed scene before fresh text is entered', async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const { rerender } = render(<WorldActionForm view={view} isPending={false} onSubmit={onSubmit} />);
    rerender(<WorldActionForm view={{ ...view, session: { ...view.session, turn_number: 2 } }} isPending={false} onSubmit={onSubmit} />);
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '从当前场景开始计划' } });
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith({
      kind: 'pursue_goal', target_id: null, intent: '从当前场景开始计划',
    }));
  });

  it('allows fresh input when the next scene arrives before a successful submission finishes', async () => {
    let finish!: () => void;
    const onSubmit = vi.fn().mockImplementationOnce(() => new Promise<void>(resolve => { finish = resolve; }))
      .mockResolvedValue(undefined);
    const page = render(<WorldActionForm view={view} isPending={false} onSubmit={onSubmit} />);
    fireEvent.click(screen.getByRole('button', { name: /^1\./ }));
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));

    const next = { ...view, session: { ...view.session, turn_number: 2 } };
    page.rerender(<WorldActionForm view={next} isPending onSubmit={onSubmit} />);
    await act(async () => finish());
    page.rerender(<WorldActionForm view={next} isPending={false} onSubmit={onSubmit} />);
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '继续观察当前场景' } });

    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));
    await waitFor(() => expect(onSubmit).toHaveBeenLastCalledWith({
      kind: 'pursue_goal', target_id: null, intent: '继续观察当前场景',
    }));
    expect(onSubmit).toHaveBeenCalledTimes(2);
  });

  it('retains click-only classifier, D20 preview, advanced targets, and pending locks', async () => {
    const advancedView = {
      ...view,
      action_suggestions_available: true,
      player: { ...view.player, rules: { mode: 'advanced', attributes: { qinggong: 12 } } },
      session: {
        ...view.session,
        game_rules: {
          attributes: [{ key: 'qinggong', label: '轻功', description: '腾挪身法' }],
          action_rules: [{ kind: 'travel', attribute_key: 'qinggong', difficulty_class: 13, description: '在复杂地形中移动' }],
        },
      },
    } as unknown as OpenWorldView;
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    vi.mocked(suggestWorldAction).mockResolvedValueOnce('travel');
    const { container, rerender } = render(<WorldActionForm view={advancedView} isPending={false} onSubmit={onSubmit} />);
    fireEvent.click(screen.getByRole('button', { name: '自由输入' }));
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '前往北塔' } });
    openAdvanced();
    fireEvent.click(screen.getByRole('button', { name: '建议行动类型' }));
    await waitFor(() => expect(screen.getByRole('status').textContent).toContain('前往地点'));
    expect(suggestWorldAction).toHaveBeenCalledOnce();
    expect(onSubmit).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: '前往地点' }));
    expect(screen.getByText('检定预览')).toBeTruthy();
    expect(screen.getByText('检定预览').parentElement?.textContent).toContain('D20 + 轻功 +1，模板基础难度 13');
    expect(screen.getByRole('option', { name: '北塔' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(true);
    fireEvent.change(screen.getByLabelText('目标'), { target: { value: 'tower' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith({
      kind: 'travel', target_id: 'tower', intent: '前往北塔',
    }));

    rerender(<WorldActionForm view={advancedView} isPending={false} isLocked onSubmit={vi.fn()} />);
    for (const control of container.querySelectorAll('input, textarea, select, button')) {
      expect(control.hasAttribute('disabled')).toBe(true);
    }
  });

  it('discards late classifier results after material scene changes, not cosmetic refetches', async () => {
    let resolve!: (kind: 'travel') => void;
    vi.mocked(suggestWorldAction).mockImplementationOnce(() => new Promise(settle => { resolve = settle; }));
    const first = { ...view, action_suggestions_available: true };
    const { rerender } = render(<WorldActionForm view={first} isPending={false} onSubmit={vi.fn()} />);
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '前往北塔' } });
    openAdvanced();
    fireEvent.click(screen.getByRole('button', { name: '建议行动类型' }));
    rerender(<WorldActionForm view={{ ...first, world_state: { ...first.world_state } }} isPending={false} onSubmit={vi.fn()} />);
    await act(async () => resolve('travel'));
    expect(screen.getByRole('status').textContent).toContain('前往地点');

    fireEvent.click(screen.getByRole('button', { name: '前往地点' }));
    vi.mocked(suggestWorldAction).mockImplementationOnce(() => new Promise(settle => { resolve = settle; }));
    fireEvent.click(screen.getByRole('button', { name: '建议行动类型' }));
    rerender(<WorldActionForm view={{ ...first, session: { ...first.session, turn_number: 2 } }} isPending={false} onSubmit={vi.fn()} />);
    await act(async () => resolve('travel'));
    expect(screen.queryByRole('button', { name: '前往地点' })).toBeNull();

    let resolveAfterEdit!: (kind: 'travel') => void;
    vi.mocked(suggestWorldAction).mockImplementationOnce(() => new Promise(settle => { resolveAfterEdit = settle; }));
    const second = { ...first, session: { ...first.session, turn_number: 3 } };
    rerender(<WorldActionForm view={second} isPending={false} onSubmit={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: '建议行动类型' }));
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '换一个意图' } });
    await act(async () => resolveAfterEdit('travel'));
    expect(screen.queryByRole('button', { name: '前往地点' })).toBeNull();
  });

  it('keeps advanced thread actions valid and blocks a removed thread target', async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const { rerender } = render(<WorldActionForm view={view} isPending={false} onSubmit={onSubmit} />);
    openAdvanced();
    fireEvent.change(screen.getByLabelText('行动'), { target: { value: 'advance_thread' } });
    fireEvent.change(screen.getByLabelText('目标'), { target: { value: 'siege' } });
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '继续了解围城进展' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith({
      kind: 'advance_thread', target_id: 'siege', intent: '继续了解围城进展',
    }));

    fireEvent.change(screen.getByLabelText('行动'), { target: { value: 'advance_thread' } });
    fireEvent.change(screen.getByLabelText('目标'), { target: { value: 'siege' } });
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '推进事件' } });
    const resolved = { ...view, world_state: { state: { threads: {} } } } as unknown as OpenWorldView;
    rerender(<WorldActionForm view={resolved} isPending={false} onSubmit={onSubmit} />);
    expect(screen.getByRole('button', { name: '执行行动' }).hasAttribute('disabled')).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));
    expect(onSubmit).toHaveBeenCalledOnce();
  });

  it('offers three non-assertive free-intent fallbacks with no location or targets', async () => {
    const sparse = {
      ...view,
      player: { ...view.player, location_id: null },
      session: {
        ...view.session,
        entry_context: {
          ...view.session.entry_context, locations: [], characters: [], threads: [], scheduled_events: [],
        },
        canonical_events: [],
      },
      world_state: { state: { threads: {} } },
      journal: [],
    } as unknown as OpenWorldView;
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(<WorldActionForm view={sparse} isPending={false} onSubmit={onSubmit} />);
    const group = screen.getByRole('group', { name: '场景建议' });
    expect(within(group).getAllByRole('button')).toHaveLength(3);
    expect(within(group).getByRole('button', { name: '1. 留意周围' })).toBeTruthy();
    expect(within(group).getByRole('button', { name: '2. 整理线索' })).toBeTruthy();
    expect(within(group).getByRole('button', { name: '3. 计划下一步' })).toBeTruthy();
    fireEvent.click(within(group).getByRole('button', { name: '1. 留意周围' }));
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith({
      kind: 'pursue_goal', target_id: null, intent: '留意当前周围，看看有什么值得关注。',
    }));
  });

  it('excludes remote and dead characters from advanced character targets', () => {
    const filtered = {
      ...view,
      session: {
        ...view.session,
        dead_character_ids: ['character'],
        entry_context: {
          ...view.session.entry_context,
          characters: [...view.session.entry_context.characters, { id: 'remote', name: '远方信使' }],
        },
      },
      journal: [{ turn_number: 1, transition: { events: [
        { location_id: 'gate', actor_character_ids: ['character'] },
        { location_id: 'harbor', actor_character_ids: ['remote'] },
      ] } }],
    } as unknown as OpenWorldView;
    render(<WorldActionForm view={filtered} isPending={false} onSubmit={vi.fn()} />);
    openAdvanced();
    fireEvent.change(screen.getByLabelText('行动'), { target: { value: 'converse' } });
    expect(screen.queryByRole('option', { name: '守门人' })).toBeNull();
    expect(screen.queryByRole('option', { name: '远方信使' })).toBeNull();
  });

  it('bounds suggested intent to 500 Unicode characters', () => {
    const longName = '北'.repeat(600);
    const longView = {
      ...view,
      session: {
        ...view.session,
        entry_context: { ...view.session.entry_context, locations: [{ id: 'gate', name: '旧城门' }, { id: 'tower', name: longName }] },
      },
    } as unknown as OpenWorldView;
    render(<WorldActionForm view={longView} isPending={false} onSubmit={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: /^3\./ }));
    expect(Array.from((screen.getByLabelText('你的意图') as HTMLTextAreaElement).value)).toHaveLength(500);
  });
});

// This suite retains the Simplified Chinese journey; locale tests cover the English default.
beforeLocaleTest(() => setLocale('zh-CN'));

it('changes UI language without invalidating or replacing an action draft', async () => {
  setLocale('en');
  const onSubmit = vi.fn().mockResolvedValue(undefined);
  render(<WorldActionForm view={view} isPending={false} onSubmit={onSubmit} />);
  fireEvent.click(screen.getByRole('button', { name: '2. Check on 围城仍在继续' }));
  const intent = (screen.getByLabelText('Your intent') as HTMLTextAreaElement).value;
  act(() => setLocale('zh-CN'));
  expect(screen.getByLabelText('你的意图')).toHaveProperty('value', intent);
  expect(screen.queryByText('场景已变化，请重新选择建议或确认这份草稿。')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: '执行行动' }));
  await waitFor(() => expect(onSubmit).toHaveBeenCalledWith({ kind: 'investigate', target_id: 'siege', intent }));
  expect(suggestWorldAction).not.toHaveBeenCalled();
});
