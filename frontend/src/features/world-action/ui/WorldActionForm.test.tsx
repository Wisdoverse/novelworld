import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { OpenWorldView } from '@/shared/types';
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

describe('WorldActionForm', () => {
  it('submits an action for the player against a server-provided target', async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(<WorldActionForm view={view} isPending={false} onSubmit={onSubmit} />);

    expect(screen.getByText(/行动者始终是你创建的角色“云舟”/)).toBeTruthy();
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '沿城墙寻找安全入口' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith({
      kind: 'travel',
      target_id: 'gate',
      intent: '沿城墙寻找安全入口',
    }));
  });

  it('keeps a player-authored goal independent from canonical character goals', async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(<WorldActionForm view={view} isPending={false} onSubmit={onSubmit} />);

    fireEvent.change(screen.getByLabelText('行动'), { target: { value: 'pursue_goal' } });
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '绘制自己的世界地图' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith({
      kind: 'pursue_goal',
      target_id: null,
      intent: '绘制自己的世界地图',
    }));
  });

  it('advances an open thread without claiming it is resolved', async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const threadView = {
      ...view,
      world_state: {
        state: {
          threads: { siege: { status: 'open', description: '围城仍在继续', origin: 'canon' } },
        },
      },
    } as unknown as OpenWorldView;
    render(<WorldActionForm view={threadView} isPending={false} onSubmit={onSubmit} />);

    expect(screen.queryByRole('option', { name: '解决事件线（旧版）' })).toBeNull();
    fireEvent.change(screen.getByLabelText('行动'), { target: { value: 'advance_thread' } });
    fireEvent.change(screen.getByLabelText('你的意图'), { target: { value: '回去与刘备会合' } });
    fireEvent.click(screen.getByRole('button', { name: '执行行动' }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith({
      kind: 'advance_thread',
      target_id: 'siege',
      intent: '回去与刘备会合',
    }));
  });

  it('does not offer characters killed after the entry checkpoint', () => {
    render(<WorldActionForm
      view={{ ...view, session: { ...view.session, dead_character_ids: ['character'] } }}
      isPending={false}
      onSubmit={vi.fn()}
    />);

    fireEvent.change(screen.getByLabelText('行动'), { target: { value: 'converse' } });

    expect(screen.getByText('当前世界状态没有适合此行动的目标。')).toBeTruthy();
  });
});
