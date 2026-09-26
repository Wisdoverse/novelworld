import { useState } from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import { Loader2, Sparkles, X } from 'lucide-react';
import {
  useAssociateNovelWorldSeries,
  useCreateWorldSeries,
  useNovelWorldSeries,
  useSuggestNovelWorldSeries,
  useWorldSeriesList,
} from '@/entities/novel';
import type { Novel, WorldSeries, WorldSeriesSuggestion } from '@/shared/types';
import { getApiErrorCode } from '@/shared/api/client';
import { toast } from 'sonner';

interface WorldSeriesDialogProps {
  principalId: string;
  isPrincipalCurrent: () => boolean;
  novel: Novel;
  readyNovels: Novel[];
  onClose: () => void;
}

function seriesErrorMessage(error: unknown) {
  switch (getApiErrorCode(error)) {
    case 'series_rule_source_unavailable':
      return '所选来源书还没有可用的 D20 基础规则。请先打开来源书，在进入故事页面的高级项中生成规则，再回来创建系列。';
    case 'source_already_in_series':
      return '来源书已经属于其他系列。请选择已有系列，将当前书加入其中。';
    default:
      return '操作失败，请稍后重试。';
  }
}

function suggestionMessage(result: WorldSeriesSuggestion | undefined) {
  switch (result?.status) {
    case 'suggested':
      return '系统找到可能的同系列小说。请核对建议并明确确认后再关联。';
    case 'unconfigured':
      return '管理员尚未启用系列识别。你可以手动选择已有系列或创建系列。';
    case 'uncertain':
      return '系统无法确定系列关系。请手动选择已有系列或创建系列。';
    case 'unavailable':
      return '系列识别暂不可用。你仍可手动选择已有系列或创建系列。';
    default:
      return undefined;
  }
}

export function WorldSeriesDialog({ principalId, isPrincipalCurrent, novel, readyNovels, onClose }: WorldSeriesDialogProps) {
  const seriesList = useWorldSeriesList(principalId);
  const currentSeries = useNovelWorldSeries(principalId, novel.id);
  const suggest = useSuggestNovelWorldSeries();
  const create = useCreateWorldSeries(principalId);
  const associate = useAssociateNovelWorldSeries(principalId, novel.id);
  const [suggestion, setSuggestion] = useState<WorldSeriesSuggestion>();
  const [selectedSeriesId, setSelectedSeriesId] = useState<string>();
  const [creating, setCreating] = useState(false);
  const [seriesName, setSeriesName] = useState('');
  const [seriesBackground, setSeriesBackground] = useState('');
  const [sourceNovelId, setSourceNovelId] = useState('');
  const [createdSeriesId, setCreatedSeriesId] = useState<string>();
  const [createdSeries, setCreatedSeries] = useState<WorldSeries>();
  const [saving, setSaving] = useState(false);
  const selection = selectedSeriesId ?? currentSeries.data?.id ?? '';
  const isPending = saving || suggest.isPending || create.isPending || associate.isPending;

  const ensurePrincipal = () => isPrincipalCurrent();

  const applySuggestion = (value: NonNullable<WorldSeriesSuggestion['suggestion']>) => {
    if (value.series_id && seriesList.data?.some(series => series.id === value.series_id)) {
      setSelectedSeriesId(value.series_id);
      setCreating(false);
      return;
    }
    setSeriesName(value.name);
    if (readyNovels.some(book => book.id === value.source_novel_id)) {
      setSourceNovelId(value.source_novel_id);
    }
    setCreating(true);
  };

  const runSuggestion = async () => {
    if (!ensurePrincipal()) return;
    try {
      const result = await suggest.mutateAsync(novel.id);
      if (!ensurePrincipal()) return;
      setSuggestion(result);
      if (result.status === 'suggested' && result.suggestion) {
        toast.success('已生成系列建议，请核对后确认');
      }
    } catch {
      if (ensurePrincipal()) {
        setSuggestion({ status: 'unavailable', suggestion: null });
      }
    }
  };

  const confirmAssociation = async (seriesId: string | null) => {
    if (!ensurePrincipal()) return;
    setSaving(true);
    try {
      await associate.mutateAsync(seriesId);
      if (!ensurePrincipal()) return;
      toast.success(seriesId ? `已将《${novel.title}》关联到共享系列` : `已解除《${novel.title}》的系列关联`);
      onClose();
    } catch (error) {
      if (ensurePrincipal()) toast.error(seriesErrorMessage(error));
    } finally {
      setSaving(false);
    }
  };

  const createAndAssociate = async () => {
    if (!seriesName.trim() || !seriesBackground.trim() || !sourceNovelId || !ensurePrincipal()) return;
    setSaving(true);
    try {
      let series: WorldSeries | undefined;
      if (createdSeriesId) {
        series = createdSeries ?? seriesList.data?.find(item => item.id === createdSeriesId);
      } else {
        series = await create.mutateAsync({
          name: seriesName.trim(),
          background: seriesBackground.trim(),
          source_novel_id: sourceNovelId,
        });
        setCreatedSeriesId(series.id);
        setCreatedSeries(series);
        setSelectedSeriesId(series.id);
      }
      if (!ensurePrincipal() || !series) return;
      // Creating a series explicitly adds its source book. If this novel is the
      // selected source, that confirmed create already completes the association.
      if (sourceNovelId !== novel.id) {
        await associate.mutateAsync(series.id);
      }
      if (!ensurePrincipal()) return;
      toast.success(`已创建系列并关联《${novel.title}》`);
      onClose();
    } catch (error) {
      if (ensurePrincipal()) toast.error(seriesErrorMessage(error));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog.Root open onOpenChange={open => { if (!open) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40 backdrop-blur-sm" />
        <Dialog.Content
          className="surface-card fixed left-1/2 top-1/2 z-50 max-h-[85vh] w-[calc(100%_-_2rem)] max-w-xl -translate-x-1/2 -translate-y-1/2 overflow-y-auto p-6 outline-none sm:p-8"
          aria-describedby="world-series-description"
        >
          <div className="flex items-start justify-between gap-4">
            <div>
              <Dialog.Title className="text-xl font-semibold text-[#1f1f1f]">
                关联系列 / 共享世界背景
              </Dialog.Title>
              <Dialog.Description id="world-series-description" className="mt-2 text-sm leading-6 text-[#5f6368]">
                为《{novel.title}》选择同系列背景。系统只提供建议；关联、背景和规则来源都由你确认。
              </Dialog.Description>
            </div>
            <Dialog.Close asChild>
              <button type="button" aria-label="关闭" className="rounded-full p-2 text-[#5f6368] hover:bg-[#f1f3f4]"><X size={18} /></button>
            </Dialog.Close>
          </div>

          <p className="mt-4 rounded-lg bg-[#f8fafd] p-3 text-xs leading-5 text-[#5f6368]">
            系列共享基础背景和 D20 基础规则；你的属性点、装备、阅读进度和个人世界状态保持独立。
          </p>

          <section className="mt-5 space-y-3" aria-label="系列识别建议">
            <button
              type="button"
              className="tonal-action w-full justify-center"
              disabled={isPending}
              onClick={() => void runSuggestion()}
            >
              {suggest.isPending ? <Loader2 size={15} className="animate-spin" /> : <Sparkles size={15} />}
              识别同系列
            </button>
            {suggest.isError ? <p role="alert" className="text-sm text-[#b3261e]">识别服务暂不可用，可手动选择或创建系列。</p> : null}
            {suggestion ? (
              <div className="rounded-lg border border-[#dadce0] p-3" role="status">
                <p className="text-sm text-[#3c4043]">{suggestionMessage(suggestion)}</p>
                {suggestion.status === 'suggested' && suggestion.suggestion ? (
                  <div className="mt-2 flex flex-wrap items-center justify-between gap-3 text-sm">
                    <span>
                      建议来源：《{suggestion.suggestion.book.title}》
                      {suggestion.suggestion.book.author ? ` · ${suggestion.suggestion.book.author}` : ''}
                      {suggestion.suggestion.book.genre ? ` · ${suggestion.suggestion.book.genre}` : ''}
                    </span>
                    <button
                      type="button"
                      className="text-[#0b57d0] underline"
                      onClick={() => applySuggestion(suggestion.suggestion!)}
                    >
                      {suggestion.suggestion.series_id ? '选择此系列建议' : '用此建议创建系列'}
                    </button>
                  </div>
                ) : null}
              </div>
            ) : null}
          </section>

          <section className="mt-6 space-y-3" aria-label="手动关联系列">
            <label className="block text-sm font-medium text-[#3c4043]">
              选择已有系列
              <select
                className="field-control mt-1"
                value={selection}
                disabled={isPending || Boolean(createdSeriesId) || seriesList.isLoading || !seriesList.data?.length}
                onChange={event => setSelectedSeriesId(event.target.value)}
              >
                <option value="">暂不关联</option>
                {(seriesList.data ?? []).map(series => (
                  <option key={series.id} value={series.id}>{series.name}</option>
                ))}
              </select>
            </label>
            {seriesList.isError || currentSeries.isError ? (
              <p role="alert" className="text-sm text-[#b3261e]">系列状态加载失败，请重试后再确认操作。</p>
            ) : null}
            <div className="flex flex-wrap gap-2">
              <button
                type="button"
                className="primary-action"
                disabled={isPending || Boolean(createdSeriesId) || !selection || selection === currentSeries.data?.id}
                onClick={() => void confirmAssociation(selection)}
              >
                确认关联当前书
              </button>
              {currentSeries.data ? (
                <button
                  type="button"
                  className="tonal-action"
                  disabled={isPending}
                  onClick={() => void confirmAssociation(null)}
                >
                  解除当前关联
                </button>
              ) : null}
              <button type="button" className="tonal-action" disabled={isPending} onClick={() => setCreating(value => !value)}>
                {creating ? '取消创建' : '创建系列'}
              </button>
            </div>
          </section>

          {creating ? (
            <section className="mt-5 space-y-3 rounded-xl border border-[#dadce0] p-4" aria-label="创建共享系列">
              <h3 className="text-sm font-semibold text-[#1f1f1f]">创建系列并确认关联</h3>
              <p className="text-xs leading-5 text-[#5f6368]">
                来源书和当前书将加入同一系列。基础背景由你填写；D20 基础规则沿用来源书。
              </p>
              <label className="block text-sm font-medium text-[#3c4043]">
                系列名称
                <input className="field-control mt-1" maxLength={80} value={seriesName} disabled={Boolean(createdSeriesId)} onChange={event => setSeriesName(event.target.value)} />
              </label>
              <label className="block text-sm font-medium text-[#3c4043]">
                共享世界背景（最多 2000 字）
                <textarea className="field-control mt-1 min-h-28" maxLength={2000} value={seriesBackground} disabled={Boolean(createdSeriesId)} onChange={event => setSeriesBackground(event.target.value)} />
              </label>
              <label className="block text-sm font-medium text-[#3c4043]">
                D20 规则来源书
                <select className="field-control mt-1" value={sourceNovelId} disabled={Boolean(createdSeriesId)} onChange={event => setSourceNovelId(event.target.value)}>
                  <option value="">选择一本已就绪的书</option>
                  {readyNovels.map(book => (
                    <option key={book.id} value={book.id}>{book.title}</option>
                  ))}
                </select>
              </label>
              {createdSeriesId ? (
                <p role="status" className="text-xs text-[#5f6368]">系列已创建；确认按钮会重试关联，不会重复创建。</p>
              ) : null}
              <button
                type="button"
                className="primary-action"
                disabled={isPending || !seriesName.trim() || !seriesBackground.trim() || !sourceNovelId}
                onClick={() => void createAndAssociate()}
              >
                {create.isPending || associate.isPending || saving ? <Loader2 size={15} className="animate-spin" /> : null}
                {createdSeriesId ? '确认关联当前书' : '创建系列并关联来源书与当前书'}
              </button>
            </section>
          ) : null}

          {currentSeries.data ? (
            <p className="mt-4 text-xs text-[#5f6368]" role="status">
              当前系列：{currentSeries.data.name}（背景与基础规则共享，角色和进度独立）。
            </p>
          ) : null}
          <div className="mt-6 flex justify-end">
            <Dialog.Close asChild><button type="button" className="tonal-action">完成</button></Dialog.Close>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
