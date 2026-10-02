import { translate as t, useLocale } from '@/shared/lib/i18n';
import { useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import * as Dialog from '@radix-ui/react-dialog';
import { Loader2, Sparkles, X } from 'lucide-react';
import {
  useAssociateNovelWorldSeries,
  useCommunitySeriesSuggestion,
  useConfirmWorldSeriesBackground,
  useCreateWorldSeries,
  useNovelWorldSeries,
  useSeriesBackgroundDraft,
  useSuggestNovelWorldSeries,
  useSuggestNovelWorldSeriesDeepSeek,
  useWorldSeriesBackgroundDraft,
  useWorldSeriesList,
  useWorldSeriesContribution,
  useSetWorldSeriesContribution,
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
      return t("The source book changed. Refresh your shelf and try again.");
    case 'source_already_in_series':
      return t("The source book already belongs to another series. Select that series to add the current book.");
    default:
      return t("The operation failed. Try again later.");
  }
}

function ruleGenerationErrorMessage(error: unknown) {
  switch (getApiErrorCode(error)) {
    case 'game_rule_sources_unavailable':
      return t("The source book lacks enough world rules to generate basic D20 rules. The series background remains available.");
    case 'game_rules_unavailable_at_progress':
      return t("Read the source book's first chapter before generating basic D20 rules.");
    case 'game_rule_generation_exhausted':
      return t("The source book's rule-generation limit was reached. The series background remains available.");
    default:
      return t("D20 rules are not ready. The series background remains available; check the status later.");
  }
}

function suggestionMessage(result: WorldSeriesSuggestion | undefined) {
  if (result?.method === 'community') {
    return result.status === 'suggested'
      ? t("Several readers linked these works. This suggests a grouping, not a shared world background. Review before confirming.")
      : result.status === 'unavailable'
        ? t("Reader suggestions are unavailable. You can still select or identify a series manually.")
        : t("No clear reader suggestions. You can still select or identify a series manually.");
  }
  if (result?.reason === 'local_evidence') {
    return t("Candidates are based on source evidence from the novels, not a Laya conclusion. Review and confirm the association yourself.");
  }
  if (result?.reason === 'too_many_books') {
    return t("Too many ready books for identification. Select an existing series or create one manually.");
  }
  if (result?.method === 'deepseek') {
    switch (result.reason) {
      case 'not_configured':
        return t("Configure the DeepSeek API in model settings before requesting a second opinion.");
      case 'unsupported_provider':
        return t("This configuration cannot request a series second opinion. Configure the DeepSeek API in model settings and try again.");
      case 'unknown_outcome':
        return t("This second opinion was not confirmed. No model request will be repeated; associate the books manually.");
    }
  }
  switch (result?.status) {
    case 'suggested':
      return t("Possible books in the same series were found. Review and explicitly confirm before associating them.");
    case 'unconfigured':
      return t("The administrator has not enabled series identification. Select an existing series or create one manually.");
    case 'uncertain':
      return t("The series relationship could not be determined. Select or create a series manually.");
    case 'unavailable':
      return t("Series identification is unavailable. You can still select or create a series manually.");
    case 'in_progress':
      return t("The DeepSeek second opinion is processing. You can check its result manually.");
    default:
      return undefined;
  }
}

export function WorldSeriesDialog({ principalId, isPrincipalCurrent, novel, readyNovels, onClose }: WorldSeriesDialogProps) {
  useLocale();
  const navigate = useNavigate();
  const seriesList = useWorldSeriesList(principalId);
  const currentSeries = useNovelWorldSeries(principalId, novel.id);
  const contribution = useWorldSeriesContribution(principalId, currentSeries.data?.id);
  const setContribution = useSetWorldSeriesContribution(principalId);
  const community = useCommunitySeriesSuggestion();
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
  const draftSourceId = creating ? sourceNovelId : backgroundPending?.source_novel_id ?? '';
  const backgroundSuggestion = useWorldSeriesBackgroundDraft();
  const seriesBackgroundSuggestion = useSeriesBackgroundDraft();
  const draftScope = useRef({ creating, sourceNovelId, seriesId: backgroundPending?.id });
  draftScope.current = { creating, sourceNovelId, seriesId: backgroundPending?.id };
  const suggestedBackground = backgroundSuggestion.data?.source_novel_id === draftSourceId
    ? backgroundSuggestion.data : undefined;
  const suggestionPending = backgroundSuggestion.isPending && backgroundSuggestion.variables === draftSourceId;
  const suggestionError = backgroundSuggestion.isError && backgroundSuggestion.variables === draftSourceId;
  const aggregateSuggestion = seriesBackgroundSuggestion.data?.series_id === backgroundPending?.id
    ? seriesBackgroundSuggestion.data : undefined;
  const aggregatePending = seriesBackgroundSuggestion.isPending && seriesBackgroundSuggestion.variables === backgroundPending?.id;
  const aggregateError = seriesBackgroundSuggestion.isError && seriesBackgroundSuggestion.variables === backgroundPending?.id;
  const selection = selectedSeriesId ?? currentSeries.data?.id ?? '';
  const isPending = saving || suggest.isPending || suggestDeepSeek.isPending
    || create.isPending || associate.isPending || confirmBackground.isPending || generateRules.isPending
    || suggestionPending || aggregatePending || community.isPending || setContribution.isPending;
  const canUseDeepSeek = suggestion?.method !== 'community' && (suggestion?.method === 'deepseek'
    ? suggestion.status === 'in_progress'
    : suggestion?.status === 'uncertain'
      || suggestion?.status === 'unavailable'
      || suggestion?.status === 'unconfigured');

  const ensurePrincipal = () => isPrincipalCurrent();

  const changeContribution = async (enabled: boolean) => {
    if (!currentSeries.data || !ensurePrincipal()) return;
    try {
      await setContribution.mutateAsync({ seriesId: currentSeries.data.id, enabled });
    } catch {
      if (ensurePrincipal()) {
        toast.error(t("Contribution settings are unconfirmed. Reload and review them."));
        void contribution.refetch();
      }
    }
  };

  const runCommunitySuggestion = async () => {
    if (!ensurePrincipal()) return;
    setSuggestion(undefined);
    try {
      const result = await community.mutateAsync(novel.id);
      if (ensurePrincipal()) setSuggestion(result);
    } catch {
      if (ensurePrincipal()) setSuggestion({ status: 'unavailable', method: 'community', suggestion: null });
    }
  };

  const fillSourceBackground = async () => {
    if (!sourceNovelId || !ensurePrincipal()) return;
    const selectedSource = sourceNovelId;
    const before = seriesBackground;
    try {
      const draft = await backgroundSuggestion.mutateAsync(selectedSource);
      if (!ensurePrincipal() || !draftScope.current.creating || draftScope.current.sourceNovelId !== selectedSource) return;
      setSeriesBackground(current => current === before ? draft.background : current);
    } catch {
      // Manual entry stays available when saved extraction cannot be read.
    }
  };

  const fillSeriesBackground = async () => {
    if (!backgroundPending || !ensurePrincipal()) return;
    const selectedSeries = backgroundPending.id;
    const before = backgroundDraft;
    try {
      const draft = await seriesBackgroundSuggestion.mutateAsync(selectedSeries);
      if (!ensurePrincipal() || draftScope.current.seriesId !== selectedSeries) return;
      setBackgroundDraft(current => current === before ? draft.background : current);
    } catch {
      // The reader can still compose a shared background manually.
    }
  };

  const changeSourceNovel = (nextSourceId: string) => {
    if (nextSourceId !== sourceNovelId
      && backgroundSuggestion.data?.source_novel_id === sourceNovelId
      && seriesBackground === backgroundSuggestion.data.background) {
      setSeriesBackground('');
    }
    setSourceNovelId(nextSourceId);
  };

  const applySuggestion = (value: NonNullable<WorldSeriesSuggestion['suggestion']>) => {
    if (value.series_id && seriesList.data?.some(series => series.id === value.series_id)) {
      setSelectedSeriesId(value.series_id);
      setCreating(false);
      return;
    }
    setSeriesName('');
    if (readyNovels.some(book => book.id === value.source_novel_id)) {
      changeSourceNovel(value.source_novel_id);
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
        toast.success(t("Series suggestions ready. Review before confirming."));
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
        toast.success(t("DeepSeek series suggestions ready. Review before confirming."));
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
      toast.success(seriesId ? t("Associated “{p0}” with the shared series", { p0: novel.title }) : t("Removed the series association for “{p0}”", { p0: novel.title }));
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
          ...(suggestedBackground ? { canon_model_version: suggestedBackground.canon_model_version } : {}),
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
      toast.success(t("Created a series and associated “{p0}”", { p0: novel.title }));
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
      toast.success(t("Shared world background confirmed"));
    } catch (error) {
      if (!ensurePrincipal()) return;
      if (getApiErrorCode(error) === 'series_background_conflict') {
        await Promise.all([currentSeries.refetch(), seriesList.refetch()]);
        toast.error(t("The shared background was confirmed elsewhere and can no longer be changed."));
      } else {
        toast.error(t("Background confirmation failed. Try again later."));
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
        toast.success(t("D20 rules generated from the source book and fixed to the series"));
      } else {
        toast.error(t("Source rules were generated, but the series association may have changed. Refresh and review it."));
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
                {currentSeries.data ? t("Series management") : t("Series association / shared world background")}
              </Dialog.Title>
              <Dialog.Description id="world-series-description" className="mt-2 text-sm leading-6 text-[#5f6368]">
                {currentSeries.data
                  ? t("“{p0}” belongs to “{p1}”. Shared world background: {p2}. Characters and reading progress stay independent.", { p0: novel.title, p1: currentSeries.data.name, p2: currentSeries.data.background === null ? t("Not yet confirmed") : t("Confirmed") })
                  : t("Select a series for “{p0}”. Suggestions are optional; you confirm the association, background and rule source.", { p0: novel.title })}
              </Dialog.Description>
            </div>
            <Dialog.Close asChild>
              <button type="button" aria-label={t("Close")} className="rounded-full p-2 text-[#5f6368] hover:bg-[#f1f3f4]"><X size={18} /></button>
            </Dialog.Close>
          </div>

          <p className="mt-4 rounded-lg bg-[#f8fafd] p-3 text-xs leading-5 text-[#5f6368]">
            {t("Associate books first and confirm a shared background later. Once confirmed, basic D20 rules can be generated from the source book. Attribute points, equipment, reading progress and personal worlds remain independent.")}
          </p>

          <section className="mt-5 space-y-3" aria-label={t("Series identification suggestions")}>
            {!currentSeries.data ? (
              <>
                <button type="button" className="tonal-action w-full justify-center" disabled={isPending || currentSeries.isLoading || currentSeries.isError} onClick={() => void runCommunitySuggestion()}>
                  {community.isPending ? <Loader2 size={15} className="animate-spin" /> : null}
                  {t("Check reader associations")}
                </button>
                <p className="text-xs leading-5 text-[#5f6368]">{t("Compares matching works from the shared library on your shelf without calling a model. Separately uploaded copies are not combined.")}</p>
              </>
            ) : null}
            <button
              type="button"
              className="tonal-action w-full justify-center"
              disabled={isPending}
              onClick={() => void runSuggestion()}
            >
              {suggest.isPending ? <Loader2 size={15} className="animate-spin" /> : <Sparkles size={15} />}
              {t("Identify related series")}
            </button>
            {suggest.isError ? <p role="alert" className="text-sm text-[#b3261e]">{t("Identification is unavailable. Select or create a series manually.")}</p> : null}
            {canUseDeepSeek ? (
              <div className="space-y-2 rounded-lg border border-[#dadce0] p-3">
                <p className="text-xs leading-5 text-[#5f6368]">
                  {t("A DeepSeek second opinion is optional and may incur model charges. Identical evidence and model settings reuse the result; failures are not automatically retried.")}
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
                    ? t("Requesting a DeepSeek second opinion")
                    : suggestDeepSeek.isError
                      ? t("Check DeepSeek second opinion")
                      : suggestion?.method === 'deepseek'
                        ? t("Check DeepSeek second opinion")
                        : t("Request DeepSeek second opinion")}
                </button>
              </div>
            ) : null}
            {suggestDeepSeek.isError ? (
              <p role="alert" className="text-sm text-[#b3261e]">
                {t("The second opinion may be unconfirmed. Checking does not start another model request. You can also choose a series manually.")}
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
                      {t("Open model settings")}
                    </button>
                  ) : null}
                {suggestion.method === 'deepseek' && suggestion.cached === true ? (
                  <p className="mt-1 text-xs text-[#5f6368]">{t("Reused a second opinion with identical evidence and model settings.")}</p>
                ) : null}
                {suggestion.status === 'suggested' && suggestion.suggestion ? (
                  <div className="mt-2 flex flex-wrap items-center justify-between gap-3 text-sm">
                    <span>
                      {t('Suggestion source: “{p0}”', { p0: suggestion.suggestion.book.title })}
                      {suggestion.suggestion.book.author ? ` · ${suggestion.suggestion.book.author}` : ''}
                      {suggestion.suggestion.book.genre ? ` · ${suggestion.suggestion.book.genre}` : ''}
                    </span>
                    <button
                      type="button"
                      className="text-[#0b57d0] underline"
                      onClick={() => applySuggestion(suggestion.suggestion!)}
                    >
                      {suggestion.suggestion.series_id ? t("Select this series suggestion") : t("Create a series from this suggestion")}
                    </button>
                  </div>
                ) : null}
              </div>
            ) : null}
          </section>

          {currentSeries.data ? (
            <section className="mt-5 space-y-2 rounded-lg border border-[#dadce0] p-3" aria-label={t("Reader association contributions")}>
              <label className="flex items-start gap-2 text-sm text-[#3c4043]">
                <input type="checkbox" className="mt-1" checked={contribution.data?.enabled ?? false} disabled={isPending || contribution.isFetching || contribution.isError || !contribution.data} onChange={event => void changeContribution(event.target.checked)} />
                {t("Use this series' book associations for reader recommendations")}
              </label>
              <p className="text-xs leading-5 text-[#5f6368]">{t("Off by default. When enabled, current and future book associations in this series join the aggregate. Series names, backgrounds and personal worlds are not shared. Disable to withdraw; unlinking, removing a book or deleting the account also removes the relevant contributions.")}</p>
              {contribution.isError ? (
                <p role="alert" className="text-xs text-[#b3261e]">{t("Contribution settings failed to load and cannot be changed yet.")}<button type="button" className="ml-2 underline" disabled={contribution.isFetching} onClick={() => void contribution.refetch()}>{t("Reload contribution settings")}</button></p>
              ) : null}
            </section>
          ) : null}

          <section className="mt-6 space-y-3" aria-label={t("Associate a series manually")}>
            <label className="block text-sm font-medium text-[#3c4043]">
              {t("Choose an existing series")}
              <select
                className="field-control mt-1"
                value={selection}
                disabled={isPending || Boolean(createdSeriesId) || seriesList.isLoading || !seriesList.data?.length}
                onChange={event => setSelectedSeriesId(event.target.value)}
              >
                <option value="">{t("Leave unassociated")}</option>
                {(seriesList.data ?? []).map(series => (
                  <option key={series.id} value={series.id}>{series.name}{series.background === null ? t("(background pending)") : series.source_template ? '' : t("(D20 rules pending)")}</option>
                ))}
              </select>
            </label>
            {seriesList.isError || currentSeries.isError ? (
              <p role="alert" className="text-sm text-[#b3261e]">{t("Series status failed to load. Retry before confirming.")}</p>
            ) : null}
            <div className="flex flex-wrap gap-2">
              <button
                type="button"
                className="primary-action"
                disabled={isPending || Boolean(createdSeriesId) || !selection || selection === currentSeries.data?.id}
                onClick={() => void confirmAssociation(selection)}
              >
                {t("Confirm association of this book")}
              </button>
              {currentSeries.data ? (
                <button
                  type="button"
                  className="tonal-action"
                  disabled={isPending}
                  onClick={() => void confirmAssociation(null)}
                >
                  {t("Remove current association")}
                </button>
              ) : null}
              <button type="button" className="tonal-action" disabled={isPending} onClick={() => setCreating(value => !value)}>
                {creating ? t("Cancel creation") : t("Create series")}
              </button>
            </div>
          </section>

          {creating ? (
            <section className="mt-5 space-y-3 rounded-xl border border-[#dadce0] p-4" aria-label={t("Create shared series")}>
              <h3 className="text-sm font-semibold text-[#1f1f1f]">{t("Create series and confirm association")}</h3>
              <p className="text-xs leading-5 text-[#5f6368]">
                {t("The source and current book will join one series. A blank background creates only a grouping. Background sharing and D20 status require confirmation. Generate basic D20 rules from the source book later.")}
              </p>
              <label className="block text-sm font-medium text-[#3c4043]">
                {t("Series name")}
                <input className="field-control mt-1" maxLength={80} value={seriesName} disabled={Boolean(createdSeriesId)} onChange={event => setSeriesName(event.target.value)} />
              </label>
              <label className="block text-sm font-medium text-[#3c4043]">
                {t("Shared world background (up to 2000 characters)")}
                <textarea className="field-control mt-1 min-h-28" maxLength={2000} value={seriesBackground} disabled={Boolean(createdSeriesId)} onChange={event => setSeriesBackground(event.target.value)} />
              </label>
              <p className="text-xs text-[#5f6368]">{t("Enter a common background or leave it blank and associate other books first. Editable until confirmed; fixed afterward. Each book retains its own characters and settings.")}</p>
              <label className="block text-sm font-medium text-[#3c4043]">
                {t("Series source book (future D20 rule source)")}
                <select className="field-control mt-1" value={sourceNovelId} disabled={Boolean(createdSeriesId)} onChange={event => changeSourceNovel(event.target.value)}>
                  <option value="">{t("Choose a ready book")}</option>
                  {readyNovels.map(book => (
                    <option key={book.id} value={book.id}>{book.title}</option>
                  ))}
                </select>
              </label>
              {sourceNovelId ? (
                <button type="button" className="tonal-action" disabled={isPending} onClick={() => void fillSourceBackground()}>
                  {t("Fill a draft from the source book (may contain spoilers)")}
                </button>
              ) : null}
              {sourceNovelId && suggestionPending ? <p className="text-xs text-[#5f6368]">{t("Loading the source book's background material…")}</p> : null}
              {sourceNovelId && suggestionError ? <p className="text-xs text-[#5f6368]">{t("No usable background suggestion from the source book. Write it manually or confirm later.")}</p> : null}
              {sourceNovelId && backgroundSuggestion.data && backgroundSuggestion.data.source_novel_id !== sourceNovelId && seriesBackground.trim() ? (
                <p role="status" className="text-xs text-[#5f6368]">{t("Source book changed. Review whether the current background still applies.")}</p>
              ) : null}
              {suggestedBackground ? (
                <p role="status" className="text-xs text-[#5f6368]">{seriesBackground === suggestedBackground.background ? t("Source book draft filled.") : t("Source book draft loaded. You have already edited the background.")}{' '}{t("This uses one book only. To include the series, create an empty background, associate other books, then return. Remove later content that should not be shared across books.")}</p>
              ) : null}
              {createdSeriesId ? (
                <p role="status" className="text-xs text-[#5f6368]">{t("Series created. Confirm retries the association without creating another series.")}</p>
              ) : null}
              <button
                type="button"
                className="primary-action"
                disabled={isPending || !seriesName.trim() || !sourceNovelId}
                onClick={() => void createAndAssociate()}
              >
                {create.isPending || associate.isPending || saving ? <Loader2 size={15} className="animate-spin" /> : null}
                {createdSeriesId ? t("Confirm association of this book") : t("Create series and associate both books")}
              </button>
            </section>
          ) : null}

          {backgroundPending ? (
            <section className="mt-4 space-y-2 rounded-lg border border-[#dadce0] p-4" aria-label={t("Confirm shared background")}>
              <p className="text-sm font-medium text-[#3c4043]">{t("This series currently groups books only. Shared background is unconfirmed and D20 status is not shown.")}</p>
              <button type="button" className="tonal-action" disabled={isPending} onClick={() => void fillSeriesBackground()}>
                {t("Combine associated novels into a background draft (may contain spoilers)")}
              </button>
              {aggregatePending ? <p className="text-xs text-[#5f6368]">{t("Loading background material from associated novels…")}</p> : null}
              {aggregateError ? <p className="text-xs text-[#5f6368]">{t("Some members lack usable parsing, or there are too many members. You can still enter the background manually.")}</p> : null}
              {aggregateSuggestion ? (
                <div className="text-xs leading-5 text-[#5f6368]" role="status">
                  <p>{backgroundDraft === aggregateSuggestion.background ? t("Filled") : t("Loaded")} {aggregateSuggestion.member_novel_ids.length} {t("associated novels. Combine common settings and remove each book's later plot details.")}</p>
                  <ol className="list-inside list-decimal">
                    {aggregateSuggestion.member_novel_ids.map(id => <li key={id}>{readyNovels.find(book => book.id === id)?.title ?? t("Member books on this shelf")}</li>)}
                  </ol>
                </div>
              ) : null}
              <label className="block text-sm font-medium text-[#3c4043]">
                {t("Shared world background (up to 2000 characters)")}
                <textarea className="field-control mt-1 min-h-28" maxLength={2000} value={backgroundDraft} onChange={event => setBackgroundDraft(event.target.value)} />
              </label>
              <p className="text-xs text-[#5f6368]">{t("Once confirmed, this cannot change. Only future stories use the shared background; existing stories keep their settings.")}</p>
              <button type="button" className="primary-action" disabled={isPending || !backgroundDraft.trim()} onClick={() => void confirmSharedBackground()}>
                {confirmBackground.isPending ? <Loader2 size={15} className="animate-spin" /> : null}
                {t("Confirm shared background")}
              </button>
            </section>
          ) : null}

          {currentSeries.data && !backgroundPending ? (
            <div className="mt-4 space-y-2 text-xs text-[#5f6368]" role="status">
              <p>{t("Current series:")}{currentSeries.data.name}{t("(shared background; independent characters and progress).")}</p>
              {pendingSeries ? (
                <>
                  <p>{t("Basic D20 rules are not yet generated for this series. Narrative mode can use the shared background first.")}</p>
                  <p>{pendingSource ? t("Rule source: “{p0}”.", { p0: pendingSource.title }) : t("The rule source book is not ready on your shelf. Rules cannot be generated yet.")}</p>
                  <p>{t("Generating rules may incur model charges. Successful source rules are fixed to this series.")}</p>
                  <button type="button" className="tonal-action" disabled={isPending || !pendingSource} onClick={() => void generateSourceRules()}>
                    {generateRules.isPending ? t("Generating source book D20 rules…") : t("Generate basic D20 rules from source book")}
                  </button>
                </>
              ) : <p>{t("Basic D20 rules are fixed to the series from its source book.")}</p>}
            </div>
          ) : null}
          <div className="mt-6 flex justify-end">
            <Dialog.Close asChild><button type="button" className="tonal-action">{t("Done")}</button></Dialog.Close>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
