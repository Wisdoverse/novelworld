import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { GameRuleTemplate } from '@/shared/types';
import { PlayerEntryForm } from './PlayerEntryForm';

const template: GameRuleTemplate = {
  novel_id: 'novel',
  canon_model_version: 3,
  schema_version: 1,
  prompt_version: 'game-rules-v1',
  minimum_score: 8,
  maximum_score: 12,
  point_budget: 20,
  attributes: [
    { key: 'qinggong', label: '轻功', description: '腾挪身法', default_score: 10, source_chapters: [1] },
    { key: 'jianghu', label: '江湖', description: '人情阅历', default_score: 10, source_chapters: [1] },
  ],
  action_rules: [],
};

const mocks = vi.hoisted(() => ({ mutate: vi.fn() }));

vi.mock('@/entities/narrative', () => ({
  useGenerateGameRules: () => ({
    mutate: mocks.mutate,
    isPending: false,
    isError: false,
    error: null,
  }),
}));

describe('PlayerEntryForm advanced rules', () => {
  beforeEach(() => {
    mocks.mutate.mockReset();
    mocks.mutate.mockImplementation((_input, options) => options.onSuccess(template));
  });

  it('allocates a shared template and submits only valid custom integer scores', async () => {
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(
      <PlayerEntryForm
        novelId="novel"
        checkpointChapter={2}
        unlockedThroughChapter={2}
        locations={[{ id: 'temple', name: '破庙' }]}
        isPending={false}
        isTimelineLocked={false}
        onCheckpointChange={vi.fn()}
        onSubmit={onSubmit}
      />,
    );

    fireEvent.click(screen.getByRole('checkbox', { name: /启用小说专属 D20/ }));
    fireEvent.click(screen.getByRole('button', { name: '生成小说专属规则' }));
    expect(mocks.mutate).toHaveBeenCalledOnce();
    expect(screen.getByText('属性点 20 / 20')).toBeTruthy();

    fireEvent.change(screen.getByLabelText('名字'), { target: { value: '燕七' } });
    fireEvent.change(screen.getByLabelText('背景'), { target: { value: '破庙里的落魄刀客' } });
    fireEvent.change(screen.getByLabelText('能力（用逗号分隔）'), { target: { value: '听风，辨穴' } });

    const scores = screen.getAllByRole('spinbutton');
    fireEvent.change(scores[0], { target: { value: '13' } });
    fireEvent.change(scores[1], { target: { value: '7' } });
    expect(screen.getByRole('button', { name: '进入故事' }).hasAttribute('disabled')).toBe(true);
    expect(screen.getByRole('alert').textContent).toContain('8–12 的整数');

    fireEvent.change(scores[0], { target: { value: '12' } });
    fireEvent.change(scores[1], { target: { value: '8' } });
    fireEvent.click(screen.getByRole('button', { name: '进入故事' }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith({
      checkpoint_chapter: 2,
      name: '燕七',
      background: '破庙里的落魄刀客',
      capabilities: ['听风', '辨穴'],
      location_id: 'temple',
      inventory: [],
      rules: {
        mode: 'advanced',
        canon_model_version: 3,
        template_schema_version: 1,
        template_prompt_version: 'game-rules-v1',
        attributes: { qinggong: 12, jianghu: 8 },
      },
    }));
  });
});
