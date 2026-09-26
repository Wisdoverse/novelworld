import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { AxiosError } from 'axios';
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

const mocks = vi.hoisted(() => ({ mutate: vi.fn(), error: null as unknown }));

vi.mock('@/entities/narrative', () => ({
  useGenerateGameRules: () => ({
    mutate: mocks.mutate,
    isPending: false,
    isError: mocks.error !== null,
    error: mocks.error,
  }),
}));

describe('PlayerEntryForm advanced rules', () => {
  beforeEach(() => {
    mocks.error = null;
    template.novel_id = 'novel';
    template.series = undefined;
    mocks.mutate.mockReset();
    mocks.mutate.mockImplementation((_input, options) => options.onSuccess(template));
  });

  it('explains locked rules and lets the reader return to narrative mode', () => {
    mocks.error = new AxiosError('Request failed', undefined, undefined, undefined, {
      status: 422,
      data: { error: { code: 'game_rules_unavailable_at_progress', message: 'upstream message' } },
    } as never);
    render(
      <PlayerEntryForm
        novelId="novel"
        checkpointChapter={1}
        unlockedThroughChapter={1}
        locations={[{ id: 'temple', name: '破庙' }]}
        isPending={false}
        isTimelineLocked={false}
        onCheckpointChange={vi.fn()}
        onSubmit={vi.fn()}
      />,
    );
    const advanced = screen.getByRole('checkbox', { name: /启用小说专属 D20/ });
    fireEvent.click(advanced);
    expect(screen.getByRole('alert').textContent).toContain('尚未解锁的章节');
    expect(screen.queryByText('upstream message')).toBeNull();
    fireEvent.click(advanced);
    expect(screen.queryByRole('alert')).toBeNull();
    expect(screen.getByRole('button', { name: '进入故事' }).hasAttribute('disabled')).toBe(false);
    expect(mocks.mutate).not.toHaveBeenCalled();
  });

  it('explains when canonical sources cannot support advanced rules', () => {
    mocks.error = new AxiosError('Request failed', undefined, undefined, undefined, {
      status: 422,
      data: { error: { code: 'game_rule_sources_unavailable', message: 'upstream message' } },
    } as never);
    render(
      <PlayerEntryForm
        novelId="novel"
        checkpointChapter={1}
        unlockedThroughChapter={1}
        locations={[{ id: 'temple', name: '破庙' }]}
        isPending={false}
        isTimelineLocked={false}
        onCheckpointChange={vi.fn()}
        onSubmit={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole('checkbox', { name: /启用小说专属 D20/ }));
    expect(screen.getByRole('alert').textContent).toBe(
      '当前小说的世界规则不足以生成基础检定。可关闭高级项，以纯叙事模式进入故事。',
    );
    expect(screen.queryByText('upstream message')).toBeNull();
  });

  it('asks the reader to wait while the novel canon is still being analyzed', () => {
    mocks.error = new AxiosError('Request failed', undefined, undefined, undefined, {
      status: 409,
      data: { error: { code: 'canon_unavailable', message: 'upstream message' } },
    } as never);
    render(
      <PlayerEntryForm
        novelId="novel"
        checkpointChapter={1}
        unlockedThroughChapter={1}
        locations={[{ id: 'temple', name: '破庙' }]}
        isPending={false}
        isTimelineLocked={false}
        onCheckpointChange={vi.fn()}
        onSubmit={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole('checkbox', { name: /启用小说专属 D20/ }));
    expect(screen.getByRole('alert').textContent).toBe(
      '小说解析尚未完成，请等待解析成功后生成规则。',
    );
    expect(screen.queryByText('upstream message')).toBeNull();
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

  it('submits the exact series binding while keeping the template source novel', async () => {
    template.novel_id = 'source-novel';
    template.series = {
      binding: { series_id: 'series-1', revision: 1 },
      target_novel_id: 'novel',
      name: '山海系列',
      background: '共享的基础世界背景',
    };
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    render(
      <PlayerEntryForm
        novelId="novel"
        checkpointChapter={1}
        unlockedThroughChapter={1}
        locations={[{ id: 'temple', name: '破庙' }]}
        isPending={false}
        isTimelineLocked={false}
        onCheckpointChange={vi.fn()}
        onSubmit={onSubmit}
      />,
    );

    fireEvent.click(screen.getByRole('checkbox', { name: /启用小说专属 D20/ }));
    fireEvent.click(screen.getByRole('button', { name: '生成小说专属规则' }));
    expect(screen.getByText('系列共享基础规则：山海系列')).toBeTruthy();
    expect(screen.getByText(/规则出处来自系列来源书。角色属性点、装备和阅读进度仍各自独立。/)).toBeTruthy();
    fireEvent.change(screen.getByLabelText('名字'), { target: { value: '燕七' } });
    fireEvent.change(screen.getByLabelText('背景'), { target: { value: '角色自己的经历' } });
    fireEvent.change(screen.getByLabelText('能力（用逗号分隔）'), { target: { value: '听风' } });
    fireEvent.click(screen.getByRole('button', { name: '进入故事' }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledWith(expect.objectContaining({
      rules: expect.objectContaining({
        mode: 'advanced',
        canon_model_version: 3,
        series_binding: { series_id: 'series-1', revision: 1 },
      }),
    })));
    expect(template.novel_id).toBe('source-novel');
  });
});
