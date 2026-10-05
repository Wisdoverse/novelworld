import { displayMessage, translate as t, useLocale, type UiMessage } from '@/shared/lib/i18n';
import { useState, useEffect, useMemo, useRef } from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import { useParams, useNavigate } from 'react-router-dom';
import { motion } from 'framer-motion';
import {
  AlertCircle, ChevronLeft, ChevronRight, MessageCircle, Users,
  BookOpen, MapPin, Sparkles, X
} from 'lucide-react';
import { useChapter, useCharacters, useNovel } from '@/entities/novel';
import {
  useReadingProgress,
  useResetReaderIdentity,
  useUpdateReadingProgress,
} from '@/entities/reading-progress';
import {
  useEffectiveChapter,
  useCreatePlayerEntity,
  useNarrativeNode,
  useOpenWorld,
  usePlayerEntry,
  useStartOpenWorld,
  useSubmitNarrativeChoice,
  useWorldState,
  isNarrativeChoiceConflict,
} from '@/entities/narrative';
import { ChatPanel } from '@/widgets/chat-panel';
import { BranchChoice } from '@/widgets/branch-choice';
import { useWorldSourceProgression } from '@/features/world-source';
import { effectiveWorldContext } from '@/shared/lib/worldSourceContext';
import { WorldDashboard } from '@/widgets/world-dashboard';
import { LanguageSwitcher } from '@/shared/ui/LanguageSwitcher';
import { PlayerEntryForm } from '@/features/player-entry';
import {
  MAX_CHAPTER_TRANSLATION_BYTES,
  TranslationControls,
  chapterTranslationByteLength,
  isChapterTranslationSupported,
  isPredominantlyChinese,
  useChapterTranslation,
} from '@/features/chapter-translation';
import { getApiErrorCode, getApiErrorMessage } from '@/shared/api/client';
import { getReaderIdentityScope } from '@/shared/lib/readerIdentityScope';
import { localWorldCharacterIds } from '@/shared/lib/localWorldCharacters';
import { prefersReducedMotion } from '@/shared/lib/reducedMotion';
import type { NarrativeChoice } from '@/shared/types';

export function splitChapterAtAnchor(content: string, anchorQuote?: string) {
  if (!anchorQuote) return { before: content, after: '', anchored: false };
  const anchorStart = content.indexOf(anchorQuote);
  if (anchorStart < 0) return { before: content, after: '', anchored: false };
  const anchorEnd = anchorStart + anchorQuote.length;
  return {
    before: content.slice(0, anchorEnd),
    after: content.slice(anchorEnd),
    anchored: true,
  };
}

function focusSection(id: string) {
  const target = document.getElementById(id);
  target?.scrollIntoView({ behavior: prefersReducedMotion() ? 'instant' : 'smooth', block: 'start' });
  target?.focus({ preventScroll: true });
}

export function ReaderPage() {
  const locale = useLocale();
  const { novelId, chapterNum } = useParams<{ novelId: string; chapterNum: string }>();
  const navigate = useNavigate();
  const {
    data: readingProgress,
    isLoading: isProgressLoading,
    isError: isProgressError,
    error: progressError,
    refetch: refetchProgress,
  } = useReadingProgress(novelId || '');
  const {
    mutate: updateCurrentChapter,
    isPending: isProgressSaving,
    isError: isProgressSaveError,
    reset: resetProgressUpdate,
  } = useUpdateReadingProgress(novelId || '');
  const resetReaderIdentity = useResetReaderIdentity(novelId || '');
  const readerIdentityUnavailable = getApiErrorCode(progressError)
    === 'reader_identity_unavailable';
  const parsedChapter = chapterNum === undefined ? undefined : Number(chapterNum);
  const routeChapter = parsedChapter !== undefined
    && Number.isInteger(parsedChapter)
    && parsedChapter >= 1
    ? parsedChapter
    : undefined;
  const currentChapter = routeChapter ?? readingProgress?.current_chapter ?? 0;
  const sourceProgression = useWorldSourceProgression({
    novelId: novelId || '', progress: readingProgress, routeChapter, navigate,
  });
  const timelineMutationLocked = sourceProgression.locked || isProgressSaving
    || readingProgress?.current_chapter !== currentChapter;
  const visibleChapterBoundary = Math.min(
    currentChapter,
    readingProgress?.current_chapter ?? 0,
  );
  const isSelfMode = readingProgress?.reader_identity_type === 'self';
  const readerIdentityScope = getReaderIdentityScope(readingProgress);
  const progressBoundary = readingProgress?.current_chapter ?? 0;

  const {
    data: novel,
    isError: isNovelError,
    refetch: refetchNovel,
  } = useNovel(novelId!);
  const {
    data: chapter,
    isLoading,
    isError: isChapterError,
    refetch: refetchChapter,
  } = useChapter(novelId!, currentChapter);
  const {
    data: effectiveChapter,
    isLoading: isEffectiveChapterLoading,
    isError: isEffectiveChapterError,
    refetch: refetchEffectiveChapter,
  } = useEffectiveChapter(
    novelId || '',
    currentChapter,
    readerIdentityScope,
    progressBoundary,
    Boolean(chapter && readingProgress && !timelineMutationLocked),
  );
  const { data: loadedCharacters } = useCharacters(
    novelId || '',
    visibleChapterBoundary,
    Boolean(routeChapter !== undefined && readingProgress && !timelineMutationLocked),
  );
  const characters = timelineMutationLocked ? undefined : loadedCharacters;
  const [entryCheckpoint, setEntryCheckpoint] = useState<number>();
  const requestedEntryCheckpoint = readingProgress
    ? Math.min(entryCheckpoint ?? readingProgress.current_chapter, readingProgress.current_chapter, currentChapter)
    : undefined;
  const {
    data: playerEntry,
    isLoading: isPlayerEntryLoading,
    isError: isPlayerEntryError,
    refetch: refetchPlayerEntry,
  } = usePlayerEntry(
    novelId || '',
    Boolean(readingProgress && isSelfMode),
    requestedEntryCheckpoint,
  );
  const createPlayerEntity = useCreatePlayerEntity(novelId || '');
  const playerEntryReady = Boolean(readingProgress)
    && (!isSelfMode || Boolean(playerEntry?.player));
  const openWorldEnabled = Boolean(isSelfMode && playerEntry?.player);
  const {
    data: cachedOpenWorld,
    isLoading: isOpenWorldLoading,
    isFetching: isOpenWorldFetching,
    isError: isOpenWorldError,
    refetch: refetchOpenWorld,
  } = useOpenWorld(novelId || '', openWorldEnabled);
  const {
    data: worldState,
    refetch: refetchWorldState,
  } = useWorldState(novelId || '', Boolean(chapter));
  const worldSourceChapters = [
    ...(isSelfMode ? [
      cachedOpenWorld?.session && effectiveWorldContext(cachedOpenWorld.session)?.unlocked_through_chapter,
      playerEntry?.player?.canonical_checkpoint_chapter,
      worldState?.state.player_entity?.canonical_checkpoint_chapter,
      worldState?.state.open_world && effectiveWorldContext(worldState.state.open_world).unlocked_through_chapter,
    ] : []),
    ...(worldState?.state.choices.map(choice => choice.chapter) ?? []),
  ].filter((chapterNumber): chapterNumber is number => (
    typeof chapterNumber === 'number'
      && Number.isInteger(chapterNumber)
      && chapterNumber >= 1
  ));
  const worldSourceHighWater = worldSourceChapters.length > 0
    ? Math.max(...worldSourceChapters)
    : undefined;
  const worldSourceVisible = worldSourceHighWater === undefined
    || visibleChapterBoundary >= worldSourceHighWater;
  const derivedTimelineVisible = !timelineMutationLocked && worldSourceVisible;
  const visibleEffectiveChapter = derivedTimelineVisible
    ? effectiveChapter
    : chapter
      ? { chapter_number: currentChapter, content: chapter.content, generated: false }
      : undefined;
  const openWorld = isSelfMode && derivedTimelineVisible ? cachedOpenWorld : null;
  const sourceRecoveryWorld = isSelfMode && sourceProgression.locked
    && worldSourceVisible && !isProgressSaving && !isProgressError
    ? cachedOpenWorld : null;
  const dashboardWorld = openWorld ?? sourceRecoveryWorld;
  const [worldActionLocked, setWorldActionLocked] = useState(false);
  useEffect(() => {
    if (openWorld && novel && !isNovelError && !isProgressError && !isProgressSaveError
      && !isOpenWorldLoading && !isOpenWorldFetching && !isOpenWorldError
      && !timelineMutationLocked && !worldActionLocked
      && openWorld.player?.id === playerEntry?.player?.id) {
      sourceProgression.advanceIfReady(openWorld, novel.total_chapters);
    }
  }, [openWorld, novel, isNovelError, isProgressError, isProgressSaveError,
    isOpenWorldLoading, isOpenWorldFetching, isOpenWorldError, timelineMutationLocked,
    worldActionLocked, playerEntry?.player?.id, sourceProgression.advanceIfReady]);
  const startOpenWorld = useStartOpenWorld(novelId || '');
  const entryLocation = playerEntry?.locations.find(
    location => location.id === playerEntry.player?.location_id,
  );

  const [activeChatCharacterId, setActiveChatCharacterId] = useState<string | null>(null);
  const [isChatOpen, setIsChatOpen] = useState(false);
  const [showCharacterList, setShowCharacterList] = useState(false);
  const characterTriggerRef = useRef<HTMLButtonElement>(null);
  const pendingChatCharacterId = useRef<string | null>(null);
  const [choiceError, setChoiceError] = useState<UiMessage | undefined>();
  const [choiceRecoveryLocked, setChoiceRecoveryLocked] = useState(false);
  const [chapterView, setChapterView] = useState<'timeline' | 'canon'>('timeline');
  const [translationEnabled, setTranslationEnabled] = useState(false);
  const lastProgressAttempt = useRef<string | undefined>(undefined);
  const hasBranch = Boolean(chapter?.is_key_node && chapter.key_node_description);
  const branchIsWithinPlayerCheckpoint = !isSelfMode
    || !playerEntry?.player
    || currentChapter <= playerEntry.player.canonical_checkpoint_chapter;
  const branchAvailableBeforeOpenWorld = !openWorldEnabled
    || (worldSourceVisible && !isOpenWorldLoading && !isOpenWorldError && !openWorld);
  const hasCommittedChoiceAtChapter = Boolean(
    derivedTimelineVisible
      && worldState?.state.choices.some(choice => choice.chapter === currentChapter),
  );
  const branchEnabled = hasBranch
    && !timelineMutationLocked
    && Boolean(visibleEffectiveChapter)
    && !isEffectiveChapterError
    && playerEntryReady
    && branchIsWithinPlayerCheckpoint
    && branchAvailableBeforeOpenWorld
    && (isSelfMode || hasCommittedChoiceAtChapter);
  const {
    data: currentBranchNode,
    isLoading: isBranchLoading,
    isError: isBranchError,
    refetch: refetchBranch,
  } = useNarrativeNode(
    novelId || '',
    currentChapter,
    branchEnabled,
  );
  const activeBranchNode = branchEnabled ? currentBranchNode : undefined;
  const visibleWorldState = derivedTimelineVisible ? worldState : undefined;
  const previousProgressChapter = useRef(readingProgress?.current_chapter);

  useEffect(() => {
    const previous = previousProgressChapter.current;
    const current = readingProgress?.current_chapter;
    previousProgressChapter.current = current;
    if (previous === undefined || current === undefined || previous === current) return;
    void refetchWorldState();
    if (openWorldEnabled) void refetchOpenWorld();
  }, [
    openWorldEnabled,
    readingProgress?.current_chapter,
    refetchOpenWorld,
    refetchWorldState,
  ]);
  useEffect(() => {
    if (sourceProgression.errorNotice) focusSection('world-source-error');
  }, [sourceProgression.errorNotice]);
  const submitChoice = useSubmitNarrativeChoice(novelId || '');

  useEffect(() => {
    if (routeChapter === undefined && readingProgress) {
      navigate(`/reader/${novelId}/${readingProgress.current_chapter}`, { replace: true });
    }
  }, [navigate, novelId, readingProgress, routeChapter]);

  useEffect(() => {
    if (routeChapter === undefined || !chapter || !readingProgress || !novelId) return;
    if (isProgressSaving || sourceProgression.locked) return;
    if (readingProgress.current_chapter === currentChapter) return;
    const attemptKey = `${novelId}:${currentChapter}`;
    if (lastProgressAttempt.current === attemptKey) return;
    lastProgressAttempt.current = attemptKey;
    updateCurrentChapter(currentChapter);
  }, [
    chapter,
    currentChapter,
    isProgressSaving,
    sourceProgression.locked,
    novelId,
    readingProgress,
    routeChapter,
    updateCurrentChapter,
  ]);

  const retryProgressUpdate = () => {
    if (!novelId || routeChapter === undefined || sourceProgression.locked) return;
    lastProgressAttempt.current = `${novelId}:${currentChapter}`;
    resetProgressUpdate();
    updateCurrentChapter(currentChapter);
  };

  const isChatReady = Boolean(
    routeChapter !== undefined
      && readingProgress
      && readerIdentityScope !== 'unresolved'
      && readingProgress.current_chapter === currentChapter
      && (!openWorldEnabled || (worldSourceVisible && !isOpenWorldLoading && !isOpenWorldError))
      && !isProgressSaving && !sourceProgression.locked,
  );
  const localCharacterIds = openWorld ? localWorldCharacterIds(openWorld) : null;
  const visibleCharacters = openWorldEnabled && (!worldSourceVisible || isOpenWorldLoading || isOpenWorldError)
    ? []
    : localCharacterIds
      ? characters?.filter(character => localCharacterIds.has(character.id))
      : characters;
  const activeChatCharacter = activeChatCharacterId
    ? visibleCharacters?.find(character => character.id === activeChatCharacterId) ?? null
    : null;
  const activeCharacterIsAvailable = Boolean(
    activeChatCharacter
      && !openWorld?.session.dead_character_ids.includes(activeChatCharacter.id),
  );

  useEffect(() => {
    if (activeChatCharacterId && visibleCharacters && !activeCharacterIsAvailable) {
      setActiveChatCharacterId(null);
    }
    if (timelineMutationLocked) setShowCharacterList(false);
  }, [
    activeCharacterIsAvailable,
    activeChatCharacterId,
    visibleCharacters,
    timelineMutationLocked,
  ]);

  useEffect(() => {
    setChoiceError(undefined);
    setChoiceRecoveryLocked(false);
    setChapterView('timeline');
    setTranslationEnabled(false);
  }, [currentChapter, novelId, readerIdentityScope]);

  useEffect(() => {
    setEntryCheckpoint(undefined);
  }, [novelId, readingProgress?.current_chapter, readingProgress?.reader_identity_type]);

  const savedChoice = activeBranchNode
    ? visibleWorldState?.state.choices.find(choice => choice.node_id === activeBranchNode.id)
    : undefined;
  const selectedChoiceIndex = savedChoice?.choice_index;
  const consequence = savedChoice?.consequence;

  useEffect(() => {
    if (choiceError && (selectedChoiceIndex !== undefined || openWorld)) {
      setChoiceError(undefined);
      setChoiceRecoveryLocked(false);
    }
  }, [choiceError, openWorld, selectedChoiceIndex]);
  const isPlayerChapter = Boolean(visibleEffectiveChapter?.generated);
  const isPlayerTimeline = Boolean(isPlayerChapter || openWorld);
  const showCanonReference = Boolean(isPlayerChapter && chapterView === 'canon');
  const isCanonReference = Boolean(showCanonReference || (openWorld && !isPlayerChapter));
  const displayContent = showCanonReference
    ? chapter?.content ?? ''
    : visibleEffectiveChapter?.content ?? '';
  const inlineChapter = chapter && activeBranchNode && !showCanonReference
    ? splitChapterAtAnchor(displayContent, activeBranchNode.anchor_quote)
    : undefined;
  const sourceContent = inlineChapter?.before ?? displayContent;
  const sourceIsChinese = useMemo(() => isPredominantlyChinese(sourceContent), [sourceContent]);
  const canOfferTranslation = Boolean(sourceContent) && !sourceIsChinese
    && (!isPlayerChapter || showCanonReference);
  const translationByteLength = chapterTranslationByteLength(sourceContent);
  const translationSupported = isChapterTranslationSupported(sourceContent);
  const canTranslate = canOfferTranslation && translationSupported;
  const translationUnavailableReason = canOfferTranslation && !translationSupported
    ? t("This chapter is {p0} bytes, exceeding the {p1}-byte translation limit. Read the original.", { p0: translationByteLength.toLocaleString(locale), p1: MAX_CHAPTER_TRANSLATION_BYTES.toLocaleString(locale) })
    : undefined;
  const translation = useChapterTranslation(
    novelId || '',
    currentChapter,
    sourceContent,
    translationEnabled && canTranslate,
  );
  const readerContent = translationEnabled && canTranslate && translation.data
    ? translation.data.content
    : sourceContent;
  const isShowingTranslation = Boolean(
    translationEnabled && canTranslate && translation.data,
  );
  const branchChoiceRequired = Boolean(
    activeBranchNode && selectedChoiceIndex === undefined && !openWorld,
  );

  const recoverCommittedChoice = async () => {
    if (!activeBranchNode) return;
    setChoiceRecoveryLocked(true);
    setChoiceError({ key: "Reloading the committed timeline…" });
    const result = await refetchWorldState();
    const committed = result.data?.state.choices.find(
      choice => choice.node_id === activeBranchNode.id,
    );
    if (committed) {
      setChoiceError(undefined);
      setChoiceRecoveryLocked(false);
      void refetchEffectiveChapter();
      return;
    }
    setChoiceError({ key: "The committed result cannot load yet. This node remains locked to avoid overwriting another window's choice." });
  };

  const handleChoose = async (choice: NarrativeChoice) => {
    if (!activeBranchNode || openWorld || !isSelfMode) return;
    setChoiceError(undefined);
    setChoiceRecoveryLocked(false);
    try {
      await submitChoice.mutateAsync({
        nodeId: activeBranchNode.id,
        choiceIndex: choice.index,
      });
    } catch (error) {
      const choiceConflict = isNarrativeChoiceConflict(error);
      setChoiceRecoveryLocked(choiceConflict);
      setChoiceError(choiceConflict
        ? { key: "Another window committed this fate node. Choices are locked while the committed result is restored." }
        : getApiErrorMessage(error, '') || { key: "Story rewrite failed. Try again." });
      throw error;
    }
  };

  const goToChapter = (num: number) => {
    if (
      timelineMutationLocked
      || num < 1
      || (novel && num > novel.total_chapters)
      || (num > currentChapter && branchChoiceRequired)
    ) return;
    navigate(`/reader/${novelId}/${num}`);
  };

  const continueJourney = () => {
    if (openWorld) {
      focusSection('world-action-form');
      return;
    }
    goToChapter(currentChapter + 1);
  };

  const goBack = () => {
    if (openWorld) {
      focusSection('world-action-journal');
      return;
    }
    goToChapter(currentChapter - 1);
  };

  if (isProgressError && !readingProgress) {
    return (
      <main className="app-surface flex min-h-screen items-center justify-center px-4 py-10">
        <div className="surface-card w-full max-w-lg px-7 py-12 text-center sm:px-10">
          <div className="mb-4 flex justify-end"><LanguageSwitcher /></div>
          <span className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-[#fce8e6] text-[#b3261e]">
            <AlertCircle size={24} aria-hidden="true" />
          </span>
          <h1 className="mt-5 text-2xl font-medium text-[#1f1f1f]">{t("Cannot open this chapter right now")}</h1>
          <p className="mt-3 text-sm leading-6 text-[#5f6368]" role="alert">{t("Reading progress could not be restored. Your records remain safe. Reload or return to your shelf.")}</p>
          <div className="mt-7 flex flex-col-reverse justify-center gap-3 sm:flex-row">
            <button className="tonal-action" onClick={() => navigate('/shelf')}>{t("Back to shelf")}</button>
            {readerIdentityUnavailable ? (
              <button
                className="primary-action"
                disabled={resetReaderIdentity.isPending}
                onClick={() => resetReaderIdentity.mutate()}
              >
                {t("Continue as yourself")}
              </button>
            ) : (
              <button className="primary-action" onClick={() => refetchProgress()}>{t("Retry")}</button>
            )}
          </div>
        </div>
      </main>
    );
  }

  if (routeChapter === undefined || currentChapter < 1 || isProgressLoading) {
    return (
      <main className="app-surface relative flex min-h-screen items-center justify-center">
        <div className="absolute right-3 top-3">
          <LanguageSwitcher />
        </div>
        <div role="status" className="h-8 w-8 animate-spin rounded-full border-2 border-[#0b57d0] border-t-transparent" aria-label={t("Restoring reading progress")} />
      </main>
    );
  }

  if ((isNovelError && !novel) || (isChapterError && !chapter)) {
    return (
      <main className="app-surface flex min-h-screen items-center justify-center px-4 py-10">
        <div className="surface-card w-full max-w-lg px-7 py-12 text-center sm:px-10" role="alert">
          <div className="mb-4 flex justify-end"><LanguageSwitcher /></div>
          <span className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-[#fce8e6] text-[#b3261e]">
            <AlertCircle size={24} aria-hidden="true" />
          </span>
          <h1 className="mt-5 text-2xl font-medium text-[#1f1f1f]">{t("Cannot load this chapter right now")}</h1>
          <p className="mt-3 text-sm leading-6 text-[#5f6368]">{t("The novel or chapter failed to load. Your reading records remain safe. Reload to try again.")}</p>
          <div className="mt-7 flex flex-col-reverse justify-center gap-3 sm:flex-row">
            <button className="tonal-action" onClick={() => navigate('/shelf')}>{t("Back to shelf")}</button>
            <button
              className="primary-action"
              onClick={() => {
                if (isNovelError) void refetchNovel();
                if (isChapterError) void refetchChapter();
              }}
            >
              {t("Retry")}
            </button>
          </div>
        </div>
      </main>
    );
  }

  const sourceProgressContent = isSelfMode
    && ((openWorld && cachedOpenWorld?.session.entry_context) || sourceProgression.locked) ? (
    <section aria-label={t("World source progress")} className="space-y-2 text-sm leading-6">
      <p className="text-[#34483d]">
        {t("World source admitted through chapter")} {cachedOpenWorld ? effectiveWorldContext(cachedOpenWorld.session).unlocked_through_chapter : sourceProgression.pending?.request.expected_source_chapter} {t(". When this scene's events finish, the world automatically continues to the next scene, preserving your character and journey.")}
      </p>
      {sourceProgression.locked ? (
        <div role="status" className="mt-3 text-sm text-[#59645f]">
          {sourceProgression.isPending ? t("Admitting the next scene…") : t("Source admission is unconfirmed. Other actions and paging are paused.")}
          <button type="button" className="tonal-action mt-3" disabled={sourceProgression.isPending} onClick={() => void sourceProgression.recover()}>
            {sourceProgression.pending?.terminal ? t("Restore latest world") : t("Continue confirming the next scene")}
          </button>
          {novel && currentChapter < novel.total_chapters ? <button type="button" className="tonal-action ml-3 mt-3" disabled={sourceProgression.isPending} onClick={() => void sourceProgression.continueOriginalReading(novel.total_chapters)}>
            {t("Read the next original chapter")}
          </button> : null}
        </div>
      ) : cachedOpenWorld && novel && effectiveWorldContext(cachedOpenWorld.session).unlocked_through_chapter < novel.total_chapters ? (
        <p className="mt-3 text-sm text-[#59645f]">{t("Keep playing this scene. Later chapters are admitted as the world progresses.")}</p>
      ) : <p className="mt-3 text-sm text-[#59645f]">{t("The original's last chapter is admitted. You can still act in this world.")}</p>}
      {worldActionLocked && !sourceProgression.locked ? <p className="mt-3 text-sm text-[#59645f]">{t("The previous action is unconfirmed. Confirm it before the next scene.")}</p> : null}
      {sourceProgression.error ? <p id="world-source-error" tabIndex={-1} role="alert" className="mt-3 text-sm text-[#b3261e]">{sourceProgression.error}</p> : null}
    </section>
  ) : null;

  return (
    <Dialog.Root open={showCharacterList && !timelineMutationLocked} onOpenChange={open => {
      if (open) setIsChatOpen(false);
      setShowCharacterList(open);
    }}>
    <div className="app-surface min-h-screen">
      {/* Top navigation */}
      <motion.header
        initial={{ y: -60 }}
        animate={{ y: 0 }}
        className="sticky top-0 z-40 flex items-center justify-between gap-3 border-b border-[#e1e3e8] bg-white/95 px-3 py-3 shadow-[0_1px_3px_rgba(60,64,67,0.08)] backdrop-blur-xl sm:px-6"
        style={{
          backdropFilter: 'blur(20px)',
        }}
      >
        <div className="flex min-w-0 items-center gap-3 sm:gap-4">
          <button
            onClick={() => navigate('/shelf')}
            className="flex shrink-0 items-center gap-1 text-sm font-medium text-[#0b57d0] transition-colors hover:text-[#0842a0] sm:gap-2"
          >
            <ChevronLeft size={16} />
            {t("Shelf")}
          </button>
          <div className="h-4 w-px shrink-0 bg-[#e1e3e8]" />
          <div className="min-w-0">
            <div className="truncate text-sm font-medium text-[#1f1f1f]">
              {novel?.title}
            </div>
            <div className="truncate text-xs text-[#5f6368]">
              {openWorld ? t("Open world · {p0}", { p0: openWorld.player?.name ?? playerEntry?.player?.name ?? t("Your character") }) : chapter?.title || t("Chapter {p0}", { p0: currentChapter })}
            </div>
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-1 sm:gap-2">
          {/* Progress */}
          <div className="hidden items-center gap-2 text-xs text-[#5f6368] md:flex">
            <BookOpen size={12} />
            {currentChapter} / {novel?.total_chapters || '?'}
          </div>

          {/* Character list button */}
          <Dialog.Trigger asChild>
          <button
            ref={characterTriggerRef}
            disabled={timelineMutationLocked}
            aria-label={t("Characters")}
            className={`flex h-11 w-10 shrink-0 items-center justify-center gap-1.5 rounded-full text-xs font-semibold transition-colors sm:w-auto sm:px-3 ${showCharacterList ? 'bg-[#d2e3fc] text-[#0842a0]' : 'bg-[#e8f0fe] text-[#0b57d0] hover:bg-[#d2e3fc]'}`}
          >
            <Users size={14} />
            <span className="hidden sm:inline">{t("Characters")}</span>
          </button>
          </Dialog.Trigger>
          <LanguageSwitcher />
        </div>
      </motion.header>

      {/* Main content */}
      <main className={`mx-auto max-w-4xl px-4 pt-1 md:px-8 ${openWorld ? 'pb-8' : 'pb-28'}`}>
        {isProgressSaveError && (
          <div className="mt-4 flex items-center justify-between gap-3 rounded-xl border border-[#f2b8b5] bg-[#fce8e6] p-3 text-[#b3261e]" role="alert">
            <span className="text-sm">{t("Reading progress failed to save. Chat is paused.")}</span>
            <button className="text-sm underline" onClick={retryProgressUpdate}>{t("Retry")}</button>
          </div>
        )}
        {isSelfMode && isPlayerEntryLoading ? (
          <p className="mt-8 text-sm text-[#5f6368]">{t("Restoring your original character…")}</p>
        ) : null}
        {isSelfMode && isPlayerEntryError ? (
          <div className="mt-8 flex items-center justify-between gap-4 rounded-xl border border-[#f2b8b5] bg-[#fce8e6] p-4 text-[#b3261e]" role="alert">
            <span className="text-sm">{t("Original character failed to load. Fate branches are paused.")}</span>
            <button className="text-sm underline" onClick={() => refetchPlayerEntry()}>{t("Retry")}</button>
          </div>
        ) : null}
        {isSelfMode && playerEntry && !playerEntry.player ? (
                  <PlayerEntryForm
                    key={novelId}
                    novelId={novelId ?? ''}
                    checkpointChapter={playerEntry.checkpoint_chapter}
            unlockedThroughChapter={readingProgress?.current_chapter ?? playerEntry.checkpoint_chapter}
            locations={playerEntry.locations}
            isPending={createPlayerEntity.isPending}
            isTimelineLocked={timelineMutationLocked}
            error={createPlayerEntity.isError
              ? getApiErrorMessage(createPlayerEntity.error, t("Original character creation failed"))
              : undefined}
            onCheckpointChange={setEntryCheckpoint}
            onSubmit={createPlayerEntity.mutateAsync}
          />
        ) : null}
        {!dashboardWorld ? sourceProgressContent : null}
        {dashboardWorld ? (
          <WorldDashboard
            novelId={novelId || ''}
            view={dashboardWorld}
            recoveryOnly={Boolean(sourceRecoveryWorld)}
            actionsDisabled={isOpenWorldError || sourceProgression.isPending || (!sourceRecoveryWorld && timelineMutationLocked)}
            actionsDisabledReason={isOpenWorldError
              ? t("Open world failed to load. The last saved journey is shown above. New actions are paused to avoid using stale state. Retry to continue.")
              : t("Reading progress is not saved yet. Wait for it to save before acting.")}
            onRefresh={refetchOpenWorld}
            onActionLockChange={setWorldActionLocked}
            onReviewJournal={openWorld ? goBack : undefined}
            sourceProgressContent={sourceProgressContent}
          />
        ) : null}
        {isLoading || (worldSourceVisible && isEffectiveChapterLoading) ? (
          <div className="flex items-center justify-center h-64">
            <div className="h-8 w-8 animate-spin rounded-full border-2 border-[#0b57d0] border-t-transparent" />
          </div>
        ) : worldSourceVisible && isEffectiveChapterError ? (
          <div
            role="alert"
            className="mt-16 flex items-center justify-between gap-4 rounded-xl border border-[#f2b8b5] bg-[#fce8e6] p-5 text-[#b3261e]"
          >
            <span className="text-sm">{t("Player timeline generation failed. This chapter is hidden to avoid falling back to obsolete source events.")}</span>
            <button className="text-sm underline" onClick={() => refetchEffectiveChapter()}>{t("Regenerate")}</button>
          </div>
        ) : chapter && visibleEffectiveChapter ? (
          <details key={openWorld ? 'world-reference' : 'reader-chapter'} open={!openWorld} className={openWorld ? 'mt-8' : ''}>
            <summary className={openWorld
              ? 'cursor-pointer rounded-2xl border border-[#ded4bf] bg-white px-5 py-4 text-sm font-semibold text-[#203a35] hover:bg-[#faf7ef]'
              : 'hidden'}>
              {t('Reading chapters and original reference · Chapter {p0}', { p0: currentChapter })}
            </summary>
          <motion.div
            key={currentChapter}
            initial={{ opacity: 0, y: 20 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.4 }}
            className="surface-card mt-6 px-6 py-8 sm:px-10 md:px-14"
          >
            {/* Chapter title */}
            <div className="text-center mb-12 pt-8">
              <div className="mb-2 text-xs font-semibold uppercase tracking-widest text-[#0b57d0]">
                {isCanonReference ? t("Original reference") : isPlayerChapter ? t("My timeline") : t("Chapter {p0}", { p0: currentChapter })}
              </div>
              {isPlayerTimeline && (
                <div className="mb-3 text-xs font-medium text-[#5f6368]">
                  {t('Original position · Chapter {p0}', { p0: currentChapter })}{chapter.title ? `《${chapter.title}》` : ''}
                </div>
              )}
              {(chapter.title || (isPlayerChapter && !showCanonReference)) && (
                <h1
                  className="text-2xl md:text-3xl font-bold"
                  style={{ color: '#1f1f1f' }}
                >
                  {isPlayerChapter && !showCanonReference
                    ? isSelfMode
                      ? t("{p0}'s story", { p0: playerEntry?.player?.name ?? t("You") })
                      : t("Character timeline")
                    : chapter.title}
                </h1>
              )}
              <div className="mx-auto mt-4 h-px w-16 bg-[#0b57d0]" />
              {visibleEffectiveChapter.generated ? (
                <div className="mx-auto mt-6 inline-flex flex-wrap justify-center rounded-full bg-[#eef3fe] p-1" role="group" aria-label={t("Reading version")}>
                  <button
                    type="button"
                    aria-pressed={!showCanonReference}
                    className={`rounded-full px-4 py-2 text-sm font-medium transition-colors ${!showCanonReference ? 'bg-white text-[#0b57d0] shadow-sm' : 'text-[#5f6368]'}`}
                    onClick={() => setChapterView('timeline')}
                  >
                    {t("My timeline")}
                  </button>
                  <button
                    type="button"
                    aria-pressed={showCanonReference}
                    className={`rounded-full px-4 py-2 text-sm font-medium transition-colors ${showCanonReference ? 'bg-white text-[#0b57d0] shadow-sm' : 'text-[#5f6368]'}`}
                    onClick={() => setChapterView('canon')}
                  >
                    {t("Original reference")}
                  </button>
                </div>
              ) : null}
              {isCanonReference ? (
                <p className="mx-auto mt-4 max-w-lg text-sm leading-6 text-[#5f6368]">
                  {t("This is source text for reviewing the world setting, not history committed in your current timeline.")}
                </p>
              ) : null}
              {canOfferTranslation ? (
                <TranslationControls
                  active={isShowingTranslation}
                  isLoading={translation.isFetching}
                  isError={translation.isError}
                  unavailableReason={translationUnavailableReason}
                  onToggle={() => {
                    if (translation.isError) {
                      void translation.refetch();
                    } else {
                      setTranslationEnabled(enabled => !enabled);
                    }
                  }}
                  onRetry={() => { void translation.refetch(); }}
                />
              ) : null}
            </div>

            {/* Inline branch: source text pauses at the anchor; generated content follows the choice. */}
            {branchEnabled && isBranchLoading && (
              <div className="my-16 flex items-center justify-center gap-2 p-5 text-sm text-[#0b57d0]">
                <div className="h-4 w-4 animate-spin rounded-full border-2 border-[#0b57d0] border-t-transparent" />
                {t("Locating this chapter's fate crossroads…")}
              </div>
            )}
            {(!branchEnabled || !isBranchLoading) && (
              <div className="reader-content" lang={isShowingTranslation || sourceIsChinese ? 'zh-CN' : undefined}>
                {readerContent.split('\n\n').map((paragraph, i) => (
                  <p key={i}>{paragraph}</p>
                ))}
              </div>
            )}
            {branchEnabled && isBranchError && (
              <div
                role="alert"
                className="my-8 flex items-center justify-between gap-4 rounded-xl border border-[#f2b8b5] bg-[#fce8e6] p-4 text-[#b3261e]"
              >
                <span className="text-sm">{t("Fate crossroads failed to load.")}</span>
                <button className="text-sm underline" onClick={() => refetchBranch()}>{t("Retry")}</button>
              </div>
            )}
            {!showCanonReference && activeBranchNode && (!openWorld || selectedChoiceIndex !== undefined) && (
              <BranchChoice
                node={activeBranchNode}
                onChoose={handleChoose}
                isLoading={submitChoice.isPending}
                selectedChoiceIndex={selectedChoiceIndex}
                consequence={consequence}
                error={displayMessage(choiceError)}
                isRecoveryLocked={choiceRecoveryLocked}
                onRetryRecovery={recoverCommittedChoice}
              />
            )}

            {/* Chapter summary */}
            {chapter.summary && !hasBranch && (!visibleEffectiveChapter.generated || showCanonReference) && (
              <div className="mt-12 rounded-xl border border-[#d2e3fc] bg-[#f8faff] p-4">
                <div className="mb-2 flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-[#0b57d0]">
                  <Sparkles size={12} />
                  {t("Chapter summary")}
                </div>
                <p className="text-sm leading-relaxed text-[#5f6368]">
                  {chapter.summary}
                </p>
              </div>
            )}
          </motion.div>
          </details>
        ) : null}
        {openWorldEnabled && isOpenWorldLoading ? (
          <p className="mt-12 text-sm text-[#5f6368]">{t("Restoring open world…")}</p>
        ) : null}
        {openWorldEnabled && !worldSourceVisible ? (
          <div className="mt-12 rounded-xl border border-[#d2e3fc] bg-[#f8faff] p-4 text-[#3c4043]" role="status">
            {t("Your reading position is behind this world's source. Read through chapter {p0} to restore actions and the journal automatically.", { p0: worldSourceHighWater })}
          </div>
        ) : null}
        {openWorldEnabled && isOpenWorldError && worldSourceVisible && !openWorld ? (
          <div className="mt-12 flex items-center justify-between gap-4 rounded-xl border border-[#f2b8b5] bg-[#fce8e6] p-4 text-[#b3261e]" role="alert">
            <span className="text-sm">{t("Open world failed to load. New actions are paused.")}</span>
            <button className="text-sm underline" onClick={() => refetchOpenWorld()}>{t("Retry")}</button>
          </div>
        ) : null}
        {openWorldEnabled
          && worldSourceVisible
          && !isOpenWorldLoading
          && !isOpenWorldError
          && !cachedOpenWorld ? (
          <section
            className="surface-card relative mt-14 overflow-hidden bg-[#f8faff] px-6 py-8 md:px-10 md:py-10"
            aria-labelledby="enter-world-title"
          >
            <div className="relative">
              <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.24em] text-[#0b57d0]">
                <Sparkles size={14} aria-hidden="true" /> {t("A new storyline")}
              </div>
              <h2
                id="enter-world-title"
                className="mt-4 max-w-xl text-2xl font-semibold leading-tight md:text-3xl"
                style={{ color: '#1f1f1f' }}
              >
                {t('Enter this world as {p0}', { p0: playerEntry?.player?.name ?? t('You') })}
              </h2>
              <p className="mt-4 max-w-xl text-sm leading-7 text-[#5f6368]">
                {t("The original is one possible path. Story characters pursue their own goals, while every action you take joins a timeline of your own.")}
              </p>
              <div className="mt-6 flex flex-wrap gap-2 text-xs text-[#3c4043]">
                <span className="rounded-full border border-[#d2e3fc] bg-[#e8f0fe] px-3 py-1.5">
                  {t('Entry · Chapter {p0}', { p0: playerEntry?.player?.canonical_checkpoint_chapter ?? currentChapter })}
                </span>
                {entryLocation ? (
                  <span className="flex items-center gap-1.5 rounded-full border border-[#d2e3fc] bg-white px-3 py-1.5">
                    <MapPin size={12} aria-hidden="true" /> {entryLocation.name}
                  </span>
                ) : null}
              </div>
              {startOpenWorld.isError ? (
                <p role="alert" className="mt-4 text-sm text-[#b3261e]">
                  {getApiErrorMessage(startOpenWorld.error, t("Entering the open world failed"))}
                </p>
              ) : null}
              <button
                className="primary-action mt-7 w-full md:w-auto"
                disabled={startOpenWorld.isPending || timelineMutationLocked}
                onClick={() => {
                  if (!timelineMutationLocked) startOpenWorld.mutate();
                }}
              >
                {startOpenWorld.isPending ? t("Creating your timeline…") : t("Enter open world")}
                {!startOpenWorld.isPending ? <ChevronRight size={16} aria-hidden="true" /> : null}
              </button>
            </div>
          </section>
        ) : null}
      </main>

      {/* Bottom paging navigation */}
      {!openWorld ? <nav aria-label={t("Reading navigation")} className="fixed bottom-0 left-0 right-0 z-20 flex items-center justify-between gap-2 border-t border-[#e1e3e8] bg-white/95 px-3 py-3 shadow-[0_-1px_3px_rgba(60,64,67,0.08)] backdrop-blur-xl sm:px-6 sm:py-4">
        <button
          onClick={goBack}
          disabled={sourceProgression.locked || isProgressSaving || (!openWorld && currentChapter <= 1)}
          className="tonal-action min-w-0 flex-1 px-3! text-xs sm:flex-none sm:px-5! sm:text-sm"
        >
          <ChevronLeft size={14} />
          {openWorld ? t("Review action journal") : t("Previous chapter")}
        </button>

        {/* Progress bar */}
        <div className="mx-4 hidden flex-1 sm:block">
          <div className="mb-1 text-center text-[11px] text-[#5f6368]">
            {isPlayerTimeline ? t("Original position ·") : ''}{currentChapter} / {novel?.total_chapters || '?'}
          </div>
          <div className="reader-progress">
            <div
              className="reader-progress-fill"
              style={{ width: `${novel ? (currentChapter / novel.total_chapters) * 100 : 0}%` }}
            />
          </div>
        </div>

        <button
          onClick={continueJourney}
          disabled={sourceProgression.locked || isProgressSaving || branchChoiceRequired || !novel || (!openWorld && currentChapter >= novel.total_chapters)}
          className="tonal-action min-w-0 flex-1 px-3! text-xs sm:flex-none sm:px-5! sm:text-sm"
        >
          {branchChoiceRequired
              ? t("Choose first")
              : openWorld
                ? t("Choose your next action")
                : isPlayerTimeline
                ? t("Continue the journey")
                : t("Next chapter")}
          <ChevronRight size={14} />
        </button>
      </nav> : null}

      {/* Character sidebar */}
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-black/20" />
        <Dialog.Content
          aria-describedby={undefined}
          onCloseAutoFocus={event => {
            // Mount chat after Radix releases the drawer's modal focus scope.
            if (pendingChatCharacterId.current) {
              event.preventDefault();
              setActiveChatCharacterId(pendingChatCharacterId.current);
              setIsChatOpen(true);
              pendingChatCharacterId.current = null;
            }
          }}
          className="fixed bottom-0 right-0 top-0 z-50 w-[min(280px,100vw)] space-y-3 overflow-y-auto border-l border-[#e1e3e8] bg-white p-4 shadow-[-8px_0_28px_rgba(60,64,67,0.1)]"
        >
          <div className="mb-4 flex items-center justify-between gap-2">
            <Dialog.Title className="text-xs font-semibold uppercase tracking-widest text-[#0b57d0]">
              {t("Story characters")}
            </Dialog.Title>
            <Dialog.Close className="tonal-action p-2" aria-label={t("Close character list")}>
              <X size={16} />
            </Dialog.Close>
          </div>
            {visibleCharacters?.length ? visibleCharacters.map((char) => {
              const isDead = openWorld?.session.dead_character_ids.includes(char.id) ?? false;
              return (
                <button
                  key={char.id}
                  disabled={!isChatReady || isDead}
                  onClick={() => {
                    if (!isChatReady || isDead) return;
                    pendingChatCharacterId.current = char.id;
                    setShowCharacterList(false);
                  }}
                  className="flex w-full items-center gap-3 rounded-xl border border-[#e1e3e8] bg-white p-3 text-left transition-colors hover:bg-[#f8faff] disabled:opacity-50"
                >
                  {char.avatar_url ? (
                    <img src={char.avatar_url} alt={char.name}
                      className="w-10 h-10 rounded-full object-cover flex-shrink-0"
                      style={{ border: '2px solid #d2e3fc' }}
                    />
                  ) : (
                    <div className="w-10 h-10 rounded-full flex-shrink-0 flex items-center justify-center font-bold"
                      style={{ background: '#0b57d0', color: 'white' }}>
                      {char.name[0]}
                    </div>
                  )}
                  <div className="min-w-0">
                    <div className="truncate text-sm font-medium text-[#1f1f1f]">{char.name}</div>
                    <div className="truncate text-xs text-[#5f6368]">
                      {isDead
                        ? t("Dead in this timeline")
                        : char.role === 'protagonist'
                          ? t("Protagonist")
                          : char.role === 'antagonist'
                            ? t("Antagonist")
                            : char.role
                              ? t("Supporting character")
                              : t("Characters")}
                    </div>
                  </div>
                  <MessageCircle size={14} className="ml-auto flex-shrink-0 text-[#0b57d0]" />
                </button>
              );
            }) : <p className="text-sm leading-6 text-[#5f6368]">
              {openWorldEnabled
                ? t("No characters are confirmed in this scene yet. Committed scene events update their presence after actions.")
                : t("No characters are visible in this chapter.")}
            </p>}
        </Dialog.Content>
      </Dialog.Portal>

      {/* Character chat panel */}
      {activeChatCharacter && activeCharacterIsAvailable && !timelineMutationLocked && (
        <ChatPanel
          key={`${novelId}:${activeChatCharacter.id}:${readerIdentityScope}:${currentChapter}`}
          character={activeChatCharacter}
          novelId={novelId!}
          currentChapter={currentChapter}
          readerIdentity={readingProgress?.reader_identity}
          readerIdentityScope={readerIdentityScope}
          canChat={isChatReady && activeCharacterIsAvailable}
          isOpen={isChatOpen}
          returnFocusRef={characterTriggerRef}
          onClose={() => setIsChatOpen(false)}
        />
      )}
    </div>
    </Dialog.Root>
  );
}
