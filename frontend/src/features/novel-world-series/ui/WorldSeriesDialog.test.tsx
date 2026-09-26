import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { WorldSeries } from '@/shared/types';
import { WorldSeriesDialog } from './WorldSeriesDialog';

const mocks = vi.hoisted(() => ({
  suggestion: vi.fn(),
  create: vi.fn(),
  associate: vi.fn(),
  userId: 'reader-1',
  suggestionResult: undefined as unknown,
  seriesList: [] as WorldSeries[],
  currentSeries: null as WorldSeries | null,
}));

vi.mock('@/entities/novel', () => ({
  useWorldSeriesList: () => ({ data: mocks.seriesList, isLoading: false, isError: false }),
  useNovelWorldSeries: () => ({ data: mocks.currentSeries, isError: false }),
  useSuggestNovelWorldSeries: () => ({
    mutateAsync: mocks.suggestion,
    isPending: false,
    isError: false,
  }),
  useCreateWorldSeries: () => ({ mutateAsync: mocks.create, isPending: false }),
  useAssociateNovelWorldSeries: () => ({ mutateAsync: mocks.associate, isPending: false }),
}));

vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

const series: WorldSeries = {
  id: 'series-1',
  name: '山海系列',
  background: '共享基础背景',
  revision: 1,
  source_template: {
    novel_id: 'source-book', canon_model_version: 2, schema_version: 1,
    prompt_version: 'novel-game-rules-v2', minimum_score: 8, maximum_score: 12,
    point_budget: 20, attributes: [], action_rules: [],
  },
  created_at: '2026-01-01T00:00:00Z',
};

const target = {
  id: 'target-book', user_id: 'reader-1', title: '当前书', total_chapters: 3,
  status: 'ready' as const, deviation_mode: 'canon' as const,
  created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z',
};
const source = { ...target, id: 'source-book', title: '来源书' };

function renderDialog() {
  return render(
    <WorldSeriesDialog
      principalId="reader-1"
      isPrincipalCurrent={() => mocks.userId === 'reader-1'}
      novel={target}
      readyNovels={[target, source]}
      onClose={vi.fn()}
    />,
  );
}

describe('WorldSeriesDialog', () => {
  beforeEach(() => {
    mocks.userId = 'reader-1';
    mocks.suggestionResult = undefined;
    mocks.seriesList = [];
    mocks.currentSeries = null;
    mocks.suggestion.mockReset();
    mocks.create.mockReset();
    mocks.associate.mockReset();
    mocks.suggestion.mockImplementation(async () => mocks.suggestionResult);
    mocks.create.mockResolvedValue(series);
    mocks.associate.mockResolvedValue(series);
  });

  it('never associates a suggested series before the reader confirms it', async () => {
    mocks.seriesList = [series];
    mocks.suggestionResult = {
      status: 'suggested',
      suggestion: {
        series_id: series.id, source_novel_id: 'source-book', name: series.name,
        book: { title: '来源书', author: '作者', genre: '奇幻' },
      },
    };
    renderDialog();

    fireEvent.click(screen.getByRole('button', { name: '识别同系列' }));
    await screen.findByText('系统找到可能的同系列小说。请核对建议并明确确认后再关联。');
    expect(mocks.associate).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole('button', { name: '选择此系列建议' }));
    expect(mocks.associate).not.toHaveBeenCalled();
    const confirmButton = screen.getAllByRole('button', { name: '确认关联当前书' });
    fireEvent.click(confirmButton[confirmButton.length - 1]);
    await waitFor(() => expect(mocks.associate).toHaveBeenCalledWith(series.id));
  });

  it('allows manual association after an uncertain suggestion', async () => {
    mocks.seriesList = [series];
    mocks.suggestionResult = { status: 'uncertain', suggestion: null };
    renderDialog();

    fireEvent.click(screen.getByRole('button', { name: '识别同系列' }));
    await screen.findByText('系统无法确定系列关系。请手动选择已有系列或创建系列。');
    fireEvent.change(screen.getByLabelText('选择已有系列'), { target: { value: series.id } });
    const confirmButton = screen.getAllByRole('button', { name: '确认关联当前书' });
    fireEvent.click(confirmButton[confirmButton.length - 1]);
    await waitFor(() => expect(mocks.associate).toHaveBeenCalledWith(series.id));
    expect(mocks.create).not.toHaveBeenCalled();
  });

  it('creates from a confirmed ready source and retries a failed target PUT without recreating', async () => {
    mocks.associate.mockRejectedValueOnce(new Error('temporary failure')).mockResolvedValueOnce(series);
    renderDialog();
    fireEvent.click(screen.getByRole('button', { name: '创建系列' }));
    fireEvent.change(screen.getByLabelText('系列名称'), { target: { value: '山海系列' } });
    fireEvent.change(screen.getByLabelText('共享世界背景（最多 2000 字）'), { target: { value: '用户确认的背景' } });
    fireEvent.change(screen.getByLabelText('D20 规则来源书'), { target: { value: 'source-book' } });
    fireEvent.click(screen.getByRole('button', { name: '创建系列并关联来源书与当前书' }));

    await screen.findByText('系列已创建；确认按钮会重试关联，不会重复创建。');
    expect(mocks.create).toHaveBeenCalledWith({
      name: '山海系列', background: '用户确认的背景', source_novel_id: 'source-book',
    });
    expect(mocks.associate).toHaveBeenCalledOnce();

    const confirmButton = screen.getAllByRole('button', { name: '确认关联当前书' });
    fireEvent.click(confirmButton[confirmButton.length - 1]);
    await waitFor(() => expect(mocks.associate).toHaveBeenCalledTimes(2));
    expect(mocks.associate).toHaveBeenLastCalledWith(series.id);
    expect(mocks.create).toHaveBeenCalledOnce();
  });

  it('lets a reader create a series from a suggested name without auto-association', async () => {
    mocks.suggestionResult = {
      status: 'suggested',
      suggestion: {
        series_id: null, source_novel_id: 'source-book', name: '建议系列名',
        book: { title: '来源书', author: null, genre: null },
      },
    };
    renderDialog();
    fireEvent.click(screen.getByRole('button', { name: '识别同系列' }));
    await screen.findByText('系统找到可能的同系列小说。请核对建议并明确确认后再关联。');
    fireEvent.click(screen.getByRole('button', { name: '用此建议创建系列' }));
    expect((screen.getByLabelText('系列名称') as HTMLInputElement).value).toBe('建议系列名');
    expect(mocks.create).not.toHaveBeenCalled();
    expect(mocks.associate).not.toHaveBeenCalled();
  });

  it('ignores a suggestion response after the active principal changes', async () => {
    mocks.suggestion.mockImplementationOnce(async () => {
      mocks.userId = 'reader-2';
      return {
        status: 'suggested',
        suggestion: {
          series_id: null, source_novel_id: 'source-book', name: 'private suggestion',
          book: { title: 'private source', author: null, genre: null },
        },
      };
    });
    renderDialog();

    fireEvent.click(screen.getByRole('button', { name: '识别同系列' }));
    await waitFor(() => expect(mocks.suggestion).toHaveBeenCalledOnce());
    expect(screen.queryByText(/private source/)).toBeNull();
    expect(mocks.associate).not.toHaveBeenCalled();
  });
});
