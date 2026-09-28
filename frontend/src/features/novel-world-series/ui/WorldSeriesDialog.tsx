import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import * as Dialog from '@radix-ui/react-dialog';
import { Loader2, Sparkles, X } from 'lucide-react';
import {
  useAssociateNovelWorldSeries,
  useConfirmWorldSeriesBackground,
  useCreateWorldSeries,
  useNovelWorldSeries,
  useSuggestNovelWorldSeries,
  useSuggestNovelWorldSeriesDeepSeek,
  useWorldSeriesList,
} from '@/entities/novel';
import { useGenerateGameRules } from '@/entities/narrative';
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
      return '来源书状态已变化，请刷新书架后重试。';
    case 'source_already_in_series':
      return '来源书已经属于其他系列。请选择已有系列，将当前书加入其中。';
    default:
      return '操作失败，请稍后重试。';
  }
}

function ruleGenerationErrorMessage(error: unknown) {
  switch (getApiErrorCode(error)) {
    case 'game_rule_sources_unavailable':
      return '来源书的世界规则不足以生成 D20 基础规则；系列背景仍可使用。';
    case 'game_rules_unavailable_at_progress':
      return '请先阅读来源书的第一章，再生成 D20 基础规则。';
    case 'game_rule_generation_exhausted':
      return '来源书的规则生成次数已用尽；系列背景仍可使用。';
    default:
      return 'D20 规则暂未生成，系列背景仍可使用。请稍后查看状态。';
  }
}

function suggestionMessage(result: WorldSeriesSuggestion | undefined) {
  if (result?.reason === 'local_evidence') {
    return '服务器依据小说原文证据给出候选；这不是 Laya 结论。请核对后手动确认关联。';
  }
  if (result?.reason === 'too_many_books') {
    return '可参与识别的已就绪书籍数量超过上限。请手动选择已有系列或创建系列。';
  }
  if (result?.method === 'deepseek') {
    switch (result.reason) {
      case 'not_configured':
        return '请先在模型设置中配置 DeepSeek API 后再补判。';
      case 'unsupported_provider':
        return '当前配置不支持系列补判。请在模型设置中配置 DeepSeek API 后再试。';
      case 'unknown_outcome':
        return '这次补判结果未确认，系统不会再次发起模型请求，请手动关联。';
    }
  }
  switch (result?.status) {
    case 'suggested':
      return '系统找到可能的同系列小说。请核对建议并明确确认后再关联。';
    case 'unconfigured':
      return '管理员尚未启用系列识别。你可以手动选择已有系列或创建系列。';
    case 'uncertain':
      return '系统无法确定系列关系。请手动选择已有系列或创建系列。';
    case 'unavailable':
      return '系列识别暂不可用。你仍可手动选择已有系列或创建系列。';
    case 'in_progress':
      return 'DeepSeek 补判正在处理中。你可以手动查询结果。';
    default:
      return undefined;
  }
}

export function WorldSeriesDialog({ principalId, isPrincipalCurrent, novel, readyNovels, onClose }: WorldSeriesDialogProps) {
  const navigate = useNavigate();
  const seriesList = useWorldSeriesList(principalId);
  const currentSeries = useNovelWorldSeries(principalId, novel.id);
  const suggest = useSuggestNovelWorldSeries();
  const suggestDeepSeek = useSuggestNovelWorldSeriesDeepSeek();
  const create = useCreateWorldSeries(principalId);
  const confirmBackground = useConfirmWorldSeriesBackground(principalId);
  const associate = useAssociateNovelWorldSeries(principalId, novel.id);
  const backgroundPending = currentSeries.data?.background === null ? currentSeries.data : undefined;
  const pendingSeries = currentSeries.data?.background !== null && currentSeries.data?.source_template === null
    ? currentSeries.data : undefined;
  const pendingSource = readyNovels.find(book => book.id === pendingSeries?.source_novel_id);
  const generateRules = useGenerateGameRules(pendingSeries?.source_novel_id ?? '');
  const [suggestion, setSuggestion] = useState<WorldSeriesSuggestion>();
  const [selectedSeriesId, setSelectedSeriesId] = useState<string>();
  const [creating, setCreating] = useState(false);
  const [seriesName, setSeriesName] = useState('');
  const [seriesBackground, setSeriesBackground] = useState('');
  const [backgroundDraft, setBackgroundDraft] = useState('');
  const [sourceNovelId, setSourceNovelId] = useState('');
  const [createdSeriesId, setCreatedSeriesId] = useState<string>();
  const [createdSeries, setCreatedSeries] = useState<WorldSeries>();
  const [saving, setSaving] = useState(false);
  const selection = selectedSeriesId ?? currentSeries.data?.id ?? '';
  const isPending = saving || suggest.isPending || suggestDeepSeek.isPending
    || create.isPending || associate.isPending || confirmBackground.isPending || generateRules.isPending;
  const canUseDeepSeek = suggestion?.method === 'deepseek'
    ? suggestion.status === 'in_progress'
    : suggestion?.status === 'uncertain'
      || suggestion?.status === 'unavailable'
      || suggestion?.status === 'unconfigured';

  const ensurePrincipal = () => isPrincipalCurrent();

  const applySuggestion = (value: NonNullable<WorldSeriesSuggestion['suggestion']>) => {
    if (value.series_id && seriesList.data?.some(series => series.id === value.series_id)) {
      setSelectedSeriesId(value.series_id);
      setCreating(false);
      return;
    }
    setSeriesName('');
    if (readyNovels.some(book => book.id === value.source_novel_id)) {
      setSourceNovelId(value.source_novel_id);
    }
    setCreating(true);
  };

  const runSuggestion = async () => {
    if (!ensurePrincipal()) return;
    suggestDeepSeek.reset();
    setSuggestion(undefined);
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

  const runDeepSeekSuggestion = async (checkOnly: boolean) => {
    if (!ensurePrincipal()) return;
    try {
      const result = await suggestDeepSeek.mutateAsync({ novelId: novel.id, checkOnly });
      if (!ensurePrincipal()) return;
      setSuggestion(result);
      if (result.status === 'suggested' && result.suggestion) {
        toast.success('已生成 DeepSeek 系列建议，请核对后确认');
      }
    } catch {
      // The paid supplement is only run again after another explicit click.
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
    if (!seriesName.trim() || !sourceNovelId || !ensurePrincipal()) return;
    setSaving(true);
    try {
      let series: WorldSeries | undefined;
      if (createdSeriesId) {
        series = createdSeries ?? seriesList.data?.find(item => item.id === createdSeriesId);
      } else {
        series = await create.mutateAsync({
          name: seriesName.trim(),
          background: seriesBackground.trim() || null,
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

  const confirmSharedBackground = async () => {
    if (!backgroundPending || !backgroundDraft.trim() || !ensurePrincipal()) return;
    try {
      await confirmBackground.mutateAsync({ seriesId: backgroundPending.id, background: backgroundDraft.trim() });
      if (!ensurePrincipal()) return;
      toast.success('共享世界背景已确认');
    } catch (error) {
      if (!ensurePrincipal()) return;
      if (getApiErrorCode(error) === 'series_background_conflict') {
        await Promise.all([currentSeries.refetch(), seriesList.refetch()]);
        toast.error('共享背景已在其他页面确认，确认后不能修改。');
      } else {
        toast.error('背景确认失败，请稍后重试。');
      }
    }
  };

  const generateSourceRules = async () => {
    if (!pendingSeries || !ensurePrincipal()) return;
    try {
      const template = await generateRules.mutateAsync();
      if (!ensurePrincipal()) return;
      await Promise.all([seriesList.refetch(), currentSeries.refetch()]);
      if (!ensurePrincipal()) return;
      if (template.series?.binding.series_id === pendingSeries.id) {
        toast.success('来源书的 D20 基础规则已生成并固定到系列');
      } else {
        toast.error('来源规则已生成，但系列关联可能已变化；请刷新后核对。');
      }
    } catch (error) {
      if (ensurePrincipal()) toast.error(ruleGenerationErrorMessage(error));
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
                为《{novel.title}》选择系列。系统只提供建议；关联、共享背景和规则来源都由你确认。
              </Dialog.Description>
            </div>
            <Dialog.Close asChild>
              <button type="button" aria-label="关闭" className="rounded-full p-2 text-[#5f6368] hover:bg-[#f1f3f4]"><X size={18} /></button>
            </Dialog.Close>
          </div>

          <p className="mt-4 rounded-lg bg-[#f8fafd] p-3 text-xs leading-5 text-[#5f6368]">
            系列可先关联小说，稍后确认共享背景；D20 基础规则可在背景确认后从来源书生成。属性点、装备、阅读进度和个人世界状态保持独立。
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
            {canUseDeepSeek ? (
              <div className="space-y-2 rounded-lg border border-[#dadce0] p-3">
                <p className="text-xs leading-5 text-[#5f6368]">
                  DeepSeek 补判是可选操作，可能产生模型费用；相同证据和模型配置会复用结果，失败不会自动重试。
                </p>
                <button
                  type="button"
                  className="tonal-action w-full justify-center"
                  disabled={isPending}
                  onClick={() => void runDeepSeekSuggestion(
                    suggestDeepSeek.isError || suggestion?.method === 'deepseek',
                  )}
                >
                  {suggestDeepSeek.isPending ? <Loader2 size={15} className="animate-spin" /> : <Sparkles size={15} />}
                  {suggestDeepSeek.isPending
                    ? '正在请求 DeepSeek 补判'
                    : suggestDeepSeek.isError
                      ? '查询 DeepSeek 补判结果'
                      : suggestion?.method === 'deepseek'
                        ? '查询 DeepSeek 补判结果'
                        : '使用 DeepSeek 补判'}
                </button>
              </div>
            ) : null}
            {suggestDeepSeek.isError ? (
              <p role="alert" className="text-sm text-[#b3261e]">
                补判结果可能尚未确认。查询结果不会重新发起模型请求；也可以手动选择系列。
              </p>
            ) : null}
            {suggestion ? (
              <div className="rounded-lg border border-[#dadce0] p-3" role="status">
                <p className="text-sm text-[#3c4043]">{suggestionMessage(suggestion)}</p>
                {suggestion.method === 'deepseek'
                  && (suggestion.reason === 'not_configured'
                    || suggestion.reason === 'unsupported_provider') ? (
                    <button
                      type="button"
                      className="mt-2 text-sm text-[#0b57d0] underline"
                      onClick={() => navigate('/settings')}
                    >
                      打开模型设置
                    </button>
                  ) : null}
                {suggestion.method === 'deepseek' && suggestion.cached === true ? (
                  <p className="mt-1 text-xs text-[#5f6368]">已复用相同证据和模型配置的补判结果。</p>
                ) : null}
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
                  <option key={series.id} value={series.id}>{series.name}{series.background === null ? '（背景待确认）' : series.source_template ? '' : '（D20 待生成）'}</option>
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
                来源书和当前书将加入同一系列。背景留空时只建立分组；确认共享背景后才会共享背景或显示 D20 状态。D20 基础规则可稍后从来源书生成。
              </p>
              <label className="block text-sm font-medium text-[#3c4043]">
                系列名称
                <input className="field-control mt-1" maxLength={80} value={seriesName} disabled={Boolean(createdSeriesId)} onChange={event => setSeriesName(event.target.value)} />
              </label>
              <label className="block text-sm font-medium text-[#3c4043]">
                共享世界背景（最多 2000 字）
                <textarea className="field-control mt-1 min-h-28" maxLength={2000} value={seriesBackground} disabled={Boolean(createdSeriesId)} onChange={event => setSeriesBackground(event.target.value)} />
              </label>
              <p className="text-xs text-[#5f6368]">留空可稍后确认；现在填写并创建后即固定，不能修改。</p>
              <label className="block text-sm font-medium text-[#3c4043]">
                系列来源书（未来 D20 规则来源）
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
                disabled={isPending || !seriesName.trim() || !sourceNovelId}
                onClick={() => void createAndAssociate()}
              >
                {create.isPending || associate.isPending || saving ? <Loader2 size={15} className="animate-spin" /> : null}
                {createdSeriesId ? '确认关联当前书' : '创建系列并关联来源书与当前书'}
              </button>
            </section>
          ) : null}

          {backgroundPending ? (
            <section className="mt-4 space-y-2 rounded-lg border border-[#dadce0] p-4" aria-label="确认共享背景">
              <p className="text-sm font-medium text-[#3c4043]">当前系列仅用于分组；共享世界背景尚未确认，D20 状态也暂不显示。</p>
              <label className="block text-sm font-medium text-[#3c4043]">
                共享世界背景（最多 2000 字）
                <textarea className="field-control mt-1 min-h-28" maxLength={2000} value={backgroundDraft} onChange={event => setBackgroundDraft(event.target.value)} />
              </label>
              <p className="text-xs text-[#5f6368]">确认后不能修改；仅之后进入的故事使用共享背景，已有故事保持原有设定。</p>
              <button type="button" className="primary-action" disabled={isPending || !backgroundDraft.trim()} onClick={() => void confirmSharedBackground()}>
                {confirmBackground.isPending ? <Loader2 size={15} className="animate-spin" /> : null}
                确认共享背景
              </button>
            </section>
          ) : null}

          {currentSeries.data && !backgroundPending ? (
            <div className="mt-4 space-y-2 text-xs text-[#5f6368]" role="status">
              <p>当前系列：{currentSeries.data.name}（背景共享；角色和进度独立）。</p>
              {pendingSeries ? (
                <>
                  <p>此系列的 D20 基础规则尚未生成。纯叙事模式可先使用共享背景。</p>
                  <p>{pendingSource ? `规则来源：《${pendingSource.title}》。` : '规则来源书不在当前已就绪书架，暂不能生成。'}</p>
                  <p>点击生成可能产生模型费用；成功后会将来源书的规则固定到系列。</p>
                  <button type="button" className="tonal-action" disabled={isPending || !pendingSource} onClick={() => void generateSourceRules()}>
                    {generateRules.isPending ? '正在生成来源书 D20 规则…' : '生成来源书 D20 基础规则'}
                  </button>
                </>
              ) : <p>D20 基础规则已从来源书固定到系列。</p>}
            </div>
          ) : null}
          <div className="mt-6 flex justify-end">
            <Dialog.Close asChild><button type="button" className="tonal-action">完成</button></Dialog.Close>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
