import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { AxiosError } from 'axios';
import type { WorldSeries } from '@/shared/types';
import { WorldSeriesDialog } from './WorldSeriesDialog';

const mocks = vi.hoisted(() => ({
  suggestion: vi.fn(),
  deepSeek: vi.fn(),
  create: vi.fn(),
  confirmBackground: vi.fn(),
  associate: vi.fn(),
  generateRules: vi.fn(),
  seriesListRefetch: vi.fn(),
  currentSeriesRefetch: vi.fn(),
  userId: 'reader-1',
  suggestionResult: undefined as unknown,
  deepSeekResult: undefined as unknown,
  deepSeekPending: false,
  deepSeekError: false,
  navigate: vi.fn(),
  seriesList: [] as WorldSeries[],
  currentSeries: null as WorldSeries | null,
}));

vi.mock('@/entities/novel', () => ({
  useWorldSeriesList: () => ({ data: mocks.seriesList, isLoading: false, isError: false, refetch: mocks.seriesListRefetch }),
  useNovelWorldSeries: () => ({ data: mocks.currentSeries, isError: false, refetch: mocks.currentSeriesRefetch }),
  useSuggestNovelWorldSeries: () => ({
    mutateAsync: mocks.suggestion,
    isPending: false,
    isError: false,
  }),
  useSuggestNovelWorldSeriesDeepSeek: () => ({
    mutateAsync: mocks.deepSeek,
    reset: vi.fn(),
    isPending: mocks.deepSeekPending,
    isError: mocks.deepSeekError,
  }),
  useCreateWorldSeries: () => ({ mutateAsync: mocks.create, isPending: false }),
  useConfirmWorldSeriesBackground: () => ({ mutateAsync: mocks.confirmBackground, isPending: false }),
  useAssociateNovelWorldSeries: () => ({ mutateAsync: mocks.associate, isPending: false }),
}));

vi.mock('@/entities/narrative', () => ({
  useGenerateGameRules: () => ({ mutateAsync: mocks.generateRules, isPending: false }),
}));

vi.mock('react-router-dom', () => ({
  useNavigate: () => mocks.navigate,
}));

vi.mock('sonner', () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

const series: WorldSeries = {
  id: 'series-1',
  name: '山海系列',
  background: '共享基础背景',
  revision: 1,
  source_novel_id: 'source-book',
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

function dialogElement() {
  return (
    <WorldSeriesDialog
      principalId="reader-1"
      isPrincipalCurrent={() => mocks.userId === 'reader-1'}
      novel={target}
      readyNovels={[target, source]}
      onClose={vi.fn()}
    />
  );
}

function renderDialog() {
  return render(dialogElement());
}

describe('WorldSeriesDialog', () => {
  beforeEach(() => {
    mocks.userId = 'reader-1';
    mocks.suggestionResult = undefined;
    mocks.deepSeekResult = undefined;
    mocks.deepSeekPending = false;
    mocks.deepSeekError = false;
    mocks.navigate.mockReset();
    mocks.seriesList = [];
    mocks.currentSeries = null;
    mocks.suggestion.mockReset();
    mocks.deepSeek.mockReset();
    mocks.create.mockReset();
    mocks.confirmBackground.mockReset();
    mocks.associate.mockReset();
    mocks.generateRules.mockReset();
    mocks.seriesListRefetch.mockReset();
    mocks.currentSeriesRefetch.mockReset();
    mocks.seriesListRefetch.mockResolvedValue(undefined);
    mocks.currentSeriesRefetch.mockResolvedValue(undefined);
    mocks.suggestion.mockImplementation(async () => mocks.suggestionResult);
    mocks.deepSeek.mockImplementation(async () => mocks.deepSeekResult);
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

  it('labels server source-evidence candidates without treating them as Laya conclusions', async () => {
    mocks.suggestionResult = {
      status: 'suggested', method: 'laya', reason: 'local_evidence', cached: false,
      suggestion: {
        series_id: null, source_novel_id: 'source-book', name: '候选系列',
        book: { title: '来源书', author: null, genre: null },
      },
    };
    renderDialog();

    fireEvent.click(screen.getByRole('button', { name: '识别同系列' }));
    await screen.findByText('服务器依据小说原文证据给出候选；这不是 Laya 结论。请核对后手动确认关联。');
    expect(mocks.associate).not.toHaveBeenCalled();
    expect(mocks.create).not.toHaveBeenCalled();
  });

  it('explains an oversized ready shelf and keeps manual series options available', async () => {
    mocks.seriesList = [series];
    mocks.suggestionResult = {
      status: 'uncertain', method: 'laya', reason: 'too_many_books', cached: false,
      suggestion: null,
    };
    renderDialog();

    fireEvent.click(screen.getByRole('button', { name: '识别同系列' }));
    await screen.findByText('可参与识别的已就绪书籍数量超过上限。请手动选择已有系列或创建系列。');
    expect(screen.getByRole('button', { name: '创建系列' })).toBeTruthy();
    expect(mocks.associate).not.toHaveBeenCalled();
  });

  it('does not use an arbitrary uploaded title as a new series name', () => {
    renderDialog();
    fireEvent.click(screen.getByRole('button', { name: '创建系列' }));
    expect((screen.getByLabelText('系列名称') as HTMLInputElement).value).toBe('');
  });

  it('creates a background-only series without silently generating D20 rules', async () => {
    mocks.create.mockResolvedValueOnce({ ...series, source_template: null });
    renderDialog();
    fireEvent.click(screen.getByRole('button', { name: '创建系列' }));
    fireEvent.change(screen.getByLabelText('系列名称'), { target: { value: '共同世界' } });
    fireEvent.change(screen.getByLabelText('共享世界背景（最多 2000 字）'), { target: { value: '读者确认的背景' } });
    fireEvent.change(screen.getByLabelText('系列来源书（未来 D20 规则来源）'), { target: { value: 'source-book' } });
    fireEvent.click(screen.getByRole('button', { name: '创建系列并关联来源书与当前书' }));

    await waitFor(() => expect(mocks.associate).toHaveBeenCalledWith(series.id));
    expect(mocks.generateRules).not.toHaveBeenCalled();
  });

  it('creates a grouping with null background', async () => {
    mocks.create.mockResolvedValueOnce({ ...series, background: null, source_template: series.source_template });
    renderDialog();
    fireEvent.click(screen.getByRole('button', { name: '创建系列' }));
    fireEvent.change(screen.getByLabelText('系列名称'), { target: { value: '共同世界' } });
    fireEvent.change(screen.getByLabelText('系列来源书（未来 D20 规则来源）'), { target: { value: 'source-book' } });
    fireEvent.click(screen.getByRole('button', { name: '创建系列并关联来源书与当前书' }));
    await waitFor(() => expect(mocks.create).toHaveBeenCalledWith({
      name: '共同世界', background: null, source_novel_id: 'source-book',
    }));
  });

  it('requires explicit background confirmation before showing background or D20 readiness', async () => {
    mocks.currentSeries = { ...series, background: null };
    renderDialog();
    expect(screen.getByText(/当前系列仅用于分组/)).toBeTruthy();
    expect(screen.getByRole('button', { name: '确认共享背景' })).toBeTruthy();
    expect(screen.queryByText('D20 基础规则已从来源书固定到系列。')).toBeNull();
    fireEvent.change(screen.getByLabelText('共享世界背景（最多 2000 字）'), { target: { value: '确认后的背景' } });
    fireEvent.click(screen.getByRole('button', { name: '确认共享背景' }));
    await waitFor(() => expect(mocks.confirmBackground).toHaveBeenCalledWith({
      seriesId: series.id, background: '确认后的背景',
    }));
  });

  it('refreshes a stale pending series after another page has confirmed it', async () => {
    mocks.currentSeries = { ...series, background: null };
    mocks.confirmBackground.mockRejectedValue(new AxiosError('conflict', undefined, undefined, undefined, {
      status: 409,
      data: { error: { code: 'series_background_conflict' } },
    } as never));
    renderDialog();
    fireEvent.change(screen.getByLabelText('共享世界背景（最多 2000 字）'), { target: { value: 'different facts' } });
    fireEvent.click(screen.getByRole('button', { name: '确认共享背景' }));
    await waitFor(() => expect(mocks.currentSeriesRefetch).toHaveBeenCalledOnce());
    expect(mocks.seriesListRefetch).toHaveBeenCalledOnce();
  });

  it('generates D20 rules only after a separate click on a pending series', async () => {
    mocks.currentSeries = { ...series, source_template: null };
    mocks.seriesList = [mocks.currentSeries];
    mocks.generateRules.mockResolvedValue({
      ...series.source_template,
      series: { binding: { series_id: series.id, revision: 1 }, target_novel_id: 'source-book', name: series.name, background: series.background },
    });
    renderDialog();

    expect(screen.getByText(/此系列的 D20 基础规则尚未生成/)).toBeTruthy();
    expect(mocks.generateRules).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: '生成来源书 D20 基础规则' }));
    await waitFor(() => expect(mocks.generateRules).toHaveBeenCalledOnce());
    expect(mocks.seriesListRefetch).toHaveBeenCalledOnce();
    expect(mocks.currentSeriesRefetch).toHaveBeenCalledOnce();
  });

  it('creates from a confirmed ready source and retries a failed target PUT without recreating', async () => {
    mocks.associate.mockRejectedValueOnce(new Error('temporary failure')).mockResolvedValueOnce(series);
    renderDialog();
    fireEvent.click(screen.getByRole('button', { name: '创建系列' }));
    fireEvent.change(screen.getByLabelText('系列名称'), { target: { value: '山海系列' } });
    fireEvent.change(screen.getByLabelText('共享世界背景（最多 2000 字）'), { target: { value: '用户确认的背景' } });
    fireEvent.change(screen.getByLabelText('系列来源书（未来 D20 规则来源）'), { target: { value: 'source-book' } });
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

  it('requires a reader-entered series name for a candidate without auto-association', async () => {
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
    expect((screen.getByLabelText('系列名称') as HTMLInputElement).value).toBe('');
    expect((screen.getByLabelText('系列来源书（未来 D20 规则来源）') as HTMLSelectElement).value).toBe('source-book');
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

  it('runs DeepSeek only after explicit consent and marks a reused result', async () => {
    mocks.suggestionResult = {
      status: 'uncertain', method: 'laya', reason: 'low_confidence', cached: false,
      suggestion: null,
    };
    mocks.deepSeekResult = {
      status: 'suggested', method: 'deepseek', reason: 'suggested', cached: true,
      suggestion: {
        series_id: null, source_novel_id: 'source-book', name: '建议系列名',
        book: { title: '来源书', author: null, genre: null },
      },
    };
    const view = renderDialog();

    fireEvent.click(screen.getByRole('button', { name: '识别同系列' }));
    await screen.findByText('系统无法确定系列关系。请手动选择已有系列或创建系列。');
    expect(mocks.deepSeek).not.toHaveBeenCalled();
    expect(screen.getByText(/可能产生模型费用/)).toBeTruthy();

    mocks.deepSeekPending = true;
    view.rerender(dialogElement());
    expect(screen.getByRole('button', { name: '正在请求 DeepSeek 补判' }).hasAttribute('disabled')).toBe(true);
    mocks.deepSeekPending = false;
    view.rerender(dialogElement());

    fireEvent.click(screen.getByRole('button', { name: '使用 DeepSeek 补判' }));
    await screen.findByText('已复用相同证据和模型配置的补判结果。');
    expect(mocks.deepSeek).toHaveBeenCalledOnce();
    expect(mocks.deepSeek).toHaveBeenCalledWith({ novelId: 'target-book', checkOnly: false });
    expect(screen.queryByText('low_confidence')).toBeNull();
    expect(mocks.associate).not.toHaveBeenCalled();
  });

  it('leaves in-progress DeepSeek work manual and only queries again on click', async () => {
    mocks.suggestionResult = { status: 'unavailable', method: 'laya', suggestion: null };
    mocks.deepSeek
      .mockResolvedValueOnce({ status: 'in_progress', method: 'deepseek', cached: false, suggestion: null })
      .mockResolvedValueOnce({
        status: 'suggested', method: 'deepseek', cached: true,
        suggestion: {
          series_id: null, source_novel_id: 'source-book', name: '建议系列名',
          book: { title: '来源书', author: null, genre: null },
        },
      });
    renderDialog();

    fireEvent.click(screen.getByRole('button', { name: '识别同系列' }));
    await screen.findByText('系列识别暂不可用。你仍可手动选择已有系列或创建系列。');
    fireEvent.click(screen.getByRole('button', { name: '使用 DeepSeek 补判' }));
    await screen.findByText('DeepSeek 补判正在处理中。你可以手动查询结果。');
    expect(mocks.deepSeek).toHaveBeenCalledOnce();
    expect(mocks.deepSeek).toHaveBeenLastCalledWith({ novelId: 'target-book', checkOnly: false });
    expect(screen.getByRole('button', { name: '查询 DeepSeek 补判结果' })).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: '查询 DeepSeek 补判结果' }));
    await screen.findByText('已复用相同证据和模型配置的补判结果。');
    expect(mocks.deepSeek).toHaveBeenCalledTimes(2);
    expect(mocks.deepSeek).toHaveBeenLastCalledWith({ novelId: 'target-book', checkOnly: true });
  });

  it('discards a DeepSeek result after the active principal changes', async () => {
    mocks.suggestionResult = { status: 'uncertain', method: 'laya', suggestion: null };
    mocks.deepSeek.mockImplementationOnce(async () => {
      mocks.userId = 'reader-2';
      return {
        status: 'suggested', method: 'deepseek', cached: false,
        suggestion: {
          series_id: null, source_novel_id: 'source-book', name: 'private suggestion',
          book: { title: 'private source', author: null, genre: null },
        },
      };
    });
    renderDialog();

    fireEvent.click(screen.getByRole('button', { name: '识别同系列' }));
    await screen.findByText('系统无法确定系列关系。请手动选择已有系列或创建系列。');
    fireEvent.click(screen.getByRole('button', { name: '使用 DeepSeek 补判' }));
    await waitFor(() => expect(mocks.deepSeek).toHaveBeenCalledOnce());
    expect(screen.queryByText(/private source/)).toBeNull();
    expect(mocks.associate).not.toHaveBeenCalled();
  });

  it.each([
    ['not_configured', '请先在模型设置中配置 DeepSeek API 后再补判。'],
    ['unsupported_provider', '当前配置不支持系列补判。请在模型设置中配置 DeepSeek API 后再试。'],
  ])('explains DeepSeek setup for the fixed %s reason', async (reason, message) => {
    mocks.suggestionResult = { status: 'uncertain', method: 'laya', suggestion: null };
    mocks.deepSeekResult = {
      status: reason === 'not_configured' ? 'unconfigured' : 'unavailable',
      method: 'deepseek', reason, cached: false, suggestion: null,
    };
    renderDialog();

    fireEvent.click(screen.getByRole('button', { name: '识别同系列' }));
    await screen.findByText('系统无法确定系列关系。请手动选择已有系列或创建系列。');
    fireEvent.click(screen.getByRole('button', { name: '使用 DeepSeek 补判' }));
    await screen.findByText(message);
    expect(mocks.deepSeek).toHaveBeenLastCalledWith({ novelId: 'target-book', checkOnly: false });

    const settings = screen.getByRole('button', { name: '打开模型设置' });
    fireEvent.click(settings);
    expect(mocks.navigate).toHaveBeenCalledWith('/settings');
    expect(screen.queryByRole('button', { name: /DeepSeek 补判/ })).toBeNull();
  });

  it('does not offer another DeepSeek request for an unknown outcome', async () => {
    mocks.suggestionResult = { status: 'uncertain', method: 'laya', suggestion: null };
    mocks.deepSeekResult = {
      status: 'unavailable', method: 'deepseek', reason: 'unknown_outcome',
      cached: false, suggestion: null,
    };
    renderDialog();

    fireEvent.click(screen.getByRole('button', { name: '识别同系列' }));
    await screen.findByText('系统无法确定系列关系。请手动选择已有系列或创建系列。');
    fireEvent.click(screen.getByRole('button', { name: '使用 DeepSeek 补判' }));
    await screen.findByText('这次补判结果未确认，系统不会再次发起模型请求，请手动关联。');
    expect(mocks.deepSeek).toHaveBeenLastCalledWith({ novelId: 'target-book', checkOnly: false });
    expect(screen.queryByRole('button', { name: /DeepSeek 补判/ })).toBeNull();
  });

  it('labels transport failure as a result query rather than a new supplement', async () => {
    mocks.suggestionResult = { status: 'uncertain', method: 'laya', suggestion: null };
    mocks.deepSeek.mockRejectedValueOnce(new Error('private transport detail'));
    const view = renderDialog();

    fireEvent.click(screen.getByRole('button', { name: '识别同系列' }));
    await screen.findByText('系统无法确定系列关系。请手动选择已有系列或创建系列。');
    fireEvent.click(screen.getByRole('button', { name: '使用 DeepSeek 补判' }));
    await waitFor(() => expect(mocks.deepSeek).toHaveBeenCalledOnce());
    expect(mocks.deepSeek).toHaveBeenLastCalledWith({ novelId: 'target-book', checkOnly: false });
    mocks.deepSeekError = true;
    view.rerender(dialogElement());
    fireEvent.click(screen.getByRole('button', { name: '查询 DeepSeek 补判结果' }));
    await screen.findByText(/补判结果可能尚未确认/);
    expect(mocks.deepSeek).toHaveBeenCalledTimes(2);
    expect(screen.queryByText(/private transport detail/)).toBeNull();
    expect(screen.queryByRole('button', { name: /重试|手动再试/ })).toBeNull();
    expect(mocks.deepSeek).toHaveBeenLastCalledWith({ novelId: 'target-book', checkOnly: true });
  });
});
