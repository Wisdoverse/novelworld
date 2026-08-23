import React, { useState, useEffect, useRef } from 'react';
import { useParams, useNavigate } from 'react-router-dom';
import { motion, AnimatePresence } from 'framer-motion';
import {
  AlertCircle, ChevronLeft, ChevronRight, MessageCircle, Users,
  BookOpen, MapPin, Sparkles
} from 'lucide-react';
import { useChapter, useCharacters, useNovel } from '@/entities/novel/api';
import {
  useReadingProgress,
  useUpdateReadingProgress,
} from '@/entities/reading-progress/api';
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
  type ChoiceResult,
} from '@/entities/narrative/api';
import { ChatPanel } from '@/widgets/chat-panel/ui/ChatPanel';
import { BranchChoice } from '@/widgets/branch-choice/ui/BranchChoice';
import { WorldDashboard } from '@/widgets/world-dashboard/ui/WorldDashboard';
import { PlayerEntryForm } from '@/features/player-entry/ui/PlayerEntryForm';
import {
  TranslationControls,
  useChapterTranslation,
} from '@/features/chapter-translation';
import { getApiErrorMessage } from '@/shared/api/client';
import type { Character, NarrativeChoice } from '@/shared/types';

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
  target?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  target?.focus({ preventScroll: true });
}

export function ReaderPage() {
  const { novelId, chapterNum } = useParams<{ novelId: string; chapterNum: string }>();
  const navigate = useNavigate();
  const {
    data: readingProgress,
    isLoading: isProgressLoading,
    isError: isProgressError,
    refetch: refetchProgress,
  } = useReadingProgress(novelId || '');
  const {
    mutate: updateCurrentChapter,
    isPending: isProgressSaving,
    isError: isProgressSaveError,
    reset: resetProgressUpdate,
  } = useUpdateReadingProgress(novelId || '');
  const parsedChapter = chapterNum === undefined ? undefined : Number(chapterNum);
  const routeChapter = parsedChapter !== undefined
    && Number.isInteger(parsedChapter)
    && parsedChapter >= 1
    ? parsedChapter
    : undefined;
  const currentChapter = routeChapter ?? readingProgress?.current_chapter ?? 0;

  const { data: novel } = useNovel(novelId!);
  const { data: chapter, isLoading } = useChapter(novelId!, currentChapter);
  const {
    data: effectiveChapter,
    isLoading: isEffectiveChapterLoading,
    isError: isEffectiveChapterError,
    refetch: refetchEffectiveChapter,
  } = useEffectiveChapter(novelId || '', currentChapter, Boolean(chapter));
  const { data: characters } = useCharacters(
    novelId || '',
    readingProgress?.current_chapter ?? 0,
  );
  const isSelfMode = readingProgress?.reader_identity_type === 'self';
  const [entryCheckpoint, setEntryCheckpoint] = useState<number>();
  const requestedEntryCheckpoint = entryCheckpoint ?? readingProgress?.current_chapter;
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
  const playerEntityRequired = Boolean(isSelfMode && !playerEntry?.player);
  const openWorldEnabled = Boolean(isSelfMode && playerEntry?.player);
  const {
    data: openWorld,
    isLoading: isOpenWorldLoading,
    isError: isOpenWorldError,
    refetch: refetchOpenWorld,
  } = useOpenWorld(novelId || '', openWorldEnabled);
  const startOpenWorld = useStartOpenWorld(novelId || '');
  const entryLocation = playerEntry?.locations.find(
    location => location.id === playerEntry.player?.location_id,
  );

  const [activeChatCharacter, setActiveChatCharacter] = useState<Character | null>(null);
  const [showCharacterList, setShowCharacterList] = useState(false);
  const [choiceResult, setChoiceResult] = useState<ChoiceResult | null>(null);
  const [choiceError, setChoiceError] = useState<string | undefined>();
  const [choiceRecoveryLocked, setChoiceRecoveryLocked] = useState(false);
  const [chapterView, setChapterView] = useState<'timeline' | 'canon'>('timeline');
  const [translationEnabled, setTranslationEnabled] = useState(false);
  const lastProgressAttempt = useRef<string | undefined>(undefined);
  const hasBranch = Boolean(chapter?.is_key_node && chapter.key_node_description);
  const {
    data: currentBranchNode,
    isLoading: isBranchLoading,
    isError: isBranchError,
    refetch: refetchBranch,
  } = useNarrativeNode(
    novelId || '',
    currentChapter,
    hasBranch && Boolean(effectiveChapter) && !isEffectiveChapterError && playerEntryReady,
  );
  const {
    data: worldState,
    refetch: refetchWorldState,
  } = useWorldState(novelId || '', Boolean(chapter));
  const submitChoice = useSubmitNarrativeChoice(novelId || '');

  useEffect(() => {
    if (routeChapter === undefined && readingProgress) {
      navigate(`/reader/${novelId}/${readingProgress.current_chapter}`, { replace: true });
    }
  }, [navigate, novelId, readingProgress, routeChapter]);

  useEffect(() => {
    if (routeChapter === undefined || !chapter || !readingProgress || !novelId) return;
    if (isProgressSaving) return;
    if (readingProgress.current_chapter === currentChapter) return;
    const attemptKey = `${novelId}:${currentChapter}`;
    if (lastProgressAttempt.current === attemptKey) return;
    lastProgressAttempt.current = attemptKey;
    updateCurrentChapter(currentChapter);
  }, [
    chapter,
    currentChapter,
    isProgressSaving,
    novelId,
    readingProgress,
    routeChapter,
    updateCurrentChapter,
  ]);

  const retryProgressUpdate = () => {
    if (!novelId || routeChapter === undefined) return;
    lastProgressAttempt.current = `${novelId}:${currentChapter}`;
    resetProgressUpdate();
    updateCurrentChapter(currentChapter);
  };

  const isChatReady = Boolean(
    routeChapter !== undefined
      && readingProgress
      && readingProgress.current_chapter === currentChapter
      && !isProgressSaving,
  );
  const activeCharacterIsAvailable = Boolean(
    activeChatCharacter
      && characters?.some(character => character.id === activeChatCharacter.id)
      && !openWorld?.session.dead_character_ids.includes(activeChatCharacter.id),
  );

  useEffect(() => {
    if (activeChatCharacter && characters && !activeCharacterIsAvailable) {
      setActiveChatCharacter(null);
    }
  }, [activeChatCharacter, activeCharacterIsAvailable, characters]);

  useEffect(() => {
    setChoiceResult(null);
    setChoiceError(undefined);
    setChoiceRecoveryLocked(false);
    setChapterView('timeline');
    setTranslationEnabled(false);
  }, [currentChapter, novelId]);

  useEffect(() => {
    setEntryCheckpoint(undefined);
  }, [novelId]);

  const savedChoice = currentBranchNode
    ? worldState?.state.choices.find(choice => choice.node_id === currentBranchNode.id)
    : undefined;
  const resultChoice = currentBranchNode
    ? choiceResult?.world_state.state.choices.find(choice => choice.node_id === currentBranchNode.id)
    : undefined;
  const selectedChoiceIndex = resultChoice?.choice_index ?? savedChoice?.choice_index;
  const consequence = choiceResult?.consequence ?? savedChoice?.consequence;

  useEffect(() => {
    if (choiceError && (selectedChoiceIndex !== undefined || openWorld)) {
      setChoiceError(undefined);
      setChoiceRecoveryLocked(false);
    }
  }, [choiceError, openWorld, selectedChoiceIndex]);
  const isPlayerChapter = Boolean(effectiveChapter?.generated);
  const isPlayerTimeline = Boolean(isPlayerChapter || openWorld);
  const showCanonReference = Boolean(isPlayerChapter && chapterView === 'canon');
  const isCanonReference = Boolean(showCanonReference || (openWorld && !isPlayerChapter));
  const displayContent = showCanonReference ? chapter?.content ?? '' : effectiveChapter?.content ?? '';
  const inlineChapter = chapter && currentBranchNode && !showCanonReference
    ? splitChapterAtAnchor(displayContent, currentBranchNode.anchor_quote)
    : undefined;
  const sourceContent = inlineChapter?.before ?? displayContent;
  const canTranslate = Boolean(sourceContent) && (!isPlayerChapter || showCanonReference);
  const translation = useChapterTranslation(
    novelId || '',
    currentChapter,
    sourceContent,
    translationEnabled && canTranslate,
  );
  const readerContent = translationEnabled && canTranslate && translation.data
    ? translation.data.content
    : sourceContent;
  const branchChoiceRequired = Boolean(
    currentBranchNode && selectedChoiceIndex === undefined && !openWorld,
  );

  const recoverCommittedChoice = async () => {
    if (!currentBranchNode) return;
    setChoiceRecoveryLocked(true);
    setChoiceError('正在重新加载已提交的时间线…');
    const result = await refetchWorldState();
    const committed = result.data?.state.choices.find(
      choice => choice.node_id === currentBranchNode.id,
    );
    if (committed) {
      setChoiceError(undefined);
      setChoiceRecoveryLocked(false);
      void refetchEffectiveChapter();
      return;
    }
    setChoiceError('已提交结果暂时无法加载；为避免覆盖另一窗口的选择，本节点仍已锁定。');
  };

  const handleChoose = async (choice: NarrativeChoice) => {
    if (!currentBranchNode || openWorld) return;
    setChoiceError(undefined);
    setChoiceRecoveryLocked(false);
    try {
      const result = await submitChoice.mutateAsync({
        nodeId: currentBranchNode.id,
        choiceIndex: choice.index,
      });
      setChoiceResult(result);
    } catch (error) {
      const choiceConflict = isNarrativeChoiceConflict(error);
      setChoiceRecoveryLocked(choiceConflict);
      setChoiceError(choiceConflict
        ? '另一窗口已经提交了这个命运节点。当前选项已锁定，正在恢复已提交的结果。'
        : getApiErrorMessage(error, '命运改写失败，请重试'));
      throw error;
    }
  };

  const goToChapter = (num: number) => {
    if (
      isProgressSaving
      || num < 1
      || (novel && num > novel.total_chapters)
      || (num > currentChapter && (branchChoiceRequired || playerEntityRequired))
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

  if (isProgressError) {
    return (
      <main className="app-surface flex min-h-screen items-center justify-center px-4 py-10">
        <div className="surface-card w-full max-w-lg px-7 py-12 text-center sm:px-10">
          <span className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-[#fce8e6] text-[#b3261e]">
            <AlertCircle size={24} aria-hidden="true" />
          </span>
          <h1 className="mt-5 text-2xl font-medium text-[#1f1f1f]">暂时无法打开本章</h1>
          <p className="mt-3 text-sm leading-6 text-[#5f6368]" role="alert">阅读进度没有成功恢复。你的阅读记录不会丢失，可以重新加载或返回书架。</p>
          <div className="mt-7 flex flex-col-reverse justify-center gap-3 sm:flex-row">
            <button className="tonal-action" onClick={() => navigate('/shelf')}>返回书架</button>
            <button className="primary-action" onClick={() => refetchProgress()}>重新加载</button>
          </div>
        </div>
      </main>
    );
  }

  if (routeChapter === undefined || currentChapter < 1 || isProgressLoading) {
    return (
      <div className="app-surface flex min-h-screen items-center justify-center">
        <div className="h-8 w-8 animate-spin rounded-full border-2 border-[#0b57d0] border-t-transparent" aria-label="正在恢复阅读进度" />
      </div>
    );
  }

  return (
    <div className="app-surface min-h-screen">
      {/* 顶部导航栏 */}
      <motion.header
        initial={{ y: -60 }}
        animate={{ y: 0 }}
        className="fixed top-0 left-0 right-0 z-40 flex items-center justify-between gap-3 border-b border-[#e1e3e8] bg-white/95 px-3 py-3 shadow-[0_1px_3px_rgba(60,64,67,0.08)] backdrop-blur-xl sm:px-6"
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
            书架
          </button>
          <div className="h-4 w-px shrink-0 bg-[#e1e3e8]" />
          <div className="min-w-0">
            <div className="truncate text-sm font-medium text-[#1f1f1f]">
              {novel?.title}
            </div>
            <div className="truncate text-xs text-[#5f6368]">
              {chapter?.title || `第 ${currentChapter} 章`}
            </div>
          </div>
        </div>

        <div className="flex items-center gap-2">
          {/* 进度 */}
          <div className="hidden items-center gap-2 text-xs text-[#5f6368] md:flex">
            <BookOpen size={12} />
            {currentChapter} / {novel?.total_chapters || '?'}
          </div>

          {/* 角色列表按钮 */}
          <button
            onClick={() => setShowCharacterList(!showCharacterList)}
            className={`flex shrink-0 items-center gap-1.5 rounded-full px-3 py-2 text-xs font-semibold transition-colors ${showCharacterList ? 'bg-[#d2e3fc] text-[#0842a0]' : 'bg-[#e8f0fe] text-[#0b57d0] hover:bg-[#d2e3fc]'}`}
          >
            <Users size={12} />
            角色
          </button>
        </div>
      </motion.header>

      {/* 主内容区 */}
      <div className="mx-auto max-w-4xl px-4 pb-28 pt-16 md:px-8">
        {isProgressSaveError && (
          <div className="mt-4 flex items-center justify-between gap-3 rounded-xl border border-[#f2b8b5] bg-[#fce8e6] p-3 text-[#b3261e]" role="alert">
            <span className="text-sm">阅读进度保存失败，聊天已暂停。</span>
            <button className="text-sm underline" onClick={retryProgressUpdate}>重试</button>
          </div>
        )}
        {isSelfMode && isPlayerEntryLoading ? (
          <p className="mt-8 text-sm text-[#5f6368]">正在恢复你的原创角色…</p>
        ) : null}
        {isSelfMode && isPlayerEntryError ? (
          <div className="mt-8 flex items-center justify-between gap-4 rounded-xl border border-[#f2b8b5] bg-[#fce8e6] p-4 text-[#b3261e]" role="alert">
            <span className="text-sm">原创角色加载失败，命运分支已暂停。</span>
            <button className="text-sm underline" onClick={() => refetchPlayerEntry()}>重试</button>
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
            error={createPlayerEntity.isError
              ? getApiErrorMessage(createPlayerEntity.error, '原创角色创建失败')
              : undefined}
            onCheckpointChange={setEntryCheckpoint}
            onSubmit={createPlayerEntity.mutateAsync}
          />
        ) : null}
        {isLoading || isEffectiveChapterLoading ? (
          <div className="flex items-center justify-center h-64">
            <div className="h-8 w-8 animate-spin rounded-full border-2 border-[#0b57d0] border-t-transparent" />
          </div>
        ) : isEffectiveChapterError ? (
          <div
            role="alert"
            className="mt-16 flex items-center justify-between gap-4 rounded-xl border border-[#f2b8b5] bg-[#fce8e6] p-5 text-[#b3261e]"
          >
            <span className="text-sm">玩家时间线生成失败。为避免回退到已经失效的原著因果，本章暂不显示。</span>
            <button className="text-sm underline" onClick={() => refetchEffectiveChapter()}>重新生成</button>
          </div>
        ) : chapter && effectiveChapter ? (
          <motion.div
            key={currentChapter}
            initial={{ opacity: 0, y: 20 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.4 }}
            className="surface-card mt-6 px-6 py-8 sm:px-10 md:px-14"
          >
            {/* 章节标题 */}
            <div className="text-center mb-12 pt-8">
              <div className="mb-2 text-xs font-semibold uppercase tracking-widest text-[#0b57d0]">
                {isCanonReference ? '原著参考' : isPlayerChapter ? '我的时间线' : `第 ${currentChapter} 章`}
              </div>
              {isPlayerTimeline && (
                <div className="mb-3 text-xs font-medium text-[#5f6368]">
                  原著坐标 · 第 {currentChapter} 章{chapter.title ? `《${chapter.title}》` : ''}
                </div>
              )}
              {(chapter.title || (isPlayerChapter && !showCanonReference)) && (
                <h1
                  className="text-2xl md:text-3xl font-bold"
                  style={{ color: '#1f1f1f' }}
                >
                  {isPlayerChapter && !showCanonReference
                    ? `${playerEntry?.player?.name ?? '你'}的故事`
                    : chapter.title}
                </h1>
              )}
              <div className="mx-auto mt-4 h-px w-16 bg-[#0b57d0]" />
              {effectiveChapter.generated ? (
                <div className="mx-auto mt-6 inline-flex rounded-full bg-[#eef3fe] p-1" aria-label="阅读版本">
                  <button
                    type="button"
                    aria-pressed={!showCanonReference}
                    className={`rounded-full px-4 py-2 text-sm font-medium transition-colors ${!showCanonReference ? 'bg-white text-[#0b57d0] shadow-sm' : 'text-[#5f6368]'}`}
                    onClick={() => setChapterView('timeline')}
                  >
                    我的时间线
                  </button>
                  <button
                    type="button"
                    aria-pressed={showCanonReference}
                    className={`rounded-full px-4 py-2 text-sm font-medium transition-colors ${showCanonReference ? 'bg-white text-[#0b57d0] shadow-sm' : 'text-[#5f6368]'}`}
                    onClick={() => setChapterView('canon')}
                  >
                    原著参考
                  </button>
                </div>
              ) : null}
              {isCanonReference ? (
                <p className="mx-auto mt-4 max-w-lg text-sm leading-6 text-[#5f6368]">
                  这是原著内容，仅用于回看世界设定，不属于你当前时间线已经发生的历史。
                </p>
              ) : null}
              {canTranslate ? (
                <TranslationControls
                  active={translationEnabled}
                  isLoading={translation.isFetching}
                  isError={translation.isError}
                  onToggle={() => setTranslationEnabled(enabled => !enabled)}
                  onRetry={() => { void translation.refetch(); }}
                />
              ) : null}
            </div>

            {/* 正文中的分支节点：原文在锚点处暂停，选择后由生成内容接续。 */}
            {hasBranch && isBranchLoading && (
              <div className="my-16 flex items-center justify-center gap-2 p-5 text-sm text-[#0b57d0]">
                <div className="h-4 w-4 animate-spin rounded-full border-2 border-[#0b57d0] border-t-transparent" />
                正在定位章节中的命运交叉点...
              </div>
            )}
            {(!hasBranch || !isBranchLoading) && (
              <div className="reader-content">
                {readerContent.split('\n\n').map((paragraph, i) => (
                  <p key={i}>{paragraph}</p>
                ))}
              </div>
            )}
            {hasBranch && isBranchError && (
              <div
                role="alert"
                className="my-8 flex items-center justify-between gap-4 rounded-xl border border-[#f2b8b5] bg-[#fce8e6] p-4 text-[#b3261e]"
              >
                <span className="text-sm">命运交叉点加载失败。</span>
                <button className="text-sm underline" onClick={() => refetchBranch()}>重试</button>
              </div>
            )}
            {!showCanonReference && currentBranchNode && (!openWorld || selectedChoiceIndex !== undefined) && (
              <BranchChoice
                node={currentBranchNode}
                onChoose={handleChoose}
                isLoading={submitChoice.isPending}
                selectedChoiceIndex={selectedChoiceIndex}
                consequence={consequence}
                error={choiceError}
                isRecoveryLocked={choiceRecoveryLocked}
                onRetryRecovery={recoverCommittedChoice}
              />
            )}

            {/* 章节摘要 */}
            {chapter.summary && !hasBranch && (!effectiveChapter.generated || showCanonReference) && (
              <div className="mt-12 rounded-xl border border-[#d2e3fc] bg-[#f8faff] p-4">
                <div className="mb-2 flex items-center gap-2 text-xs font-semibold uppercase tracking-wider text-[#0b57d0]">
                  <Sparkles size={12} />
                  章节摘要
                </div>
                <p className="text-sm leading-relaxed text-[#5f6368]">
                  {chapter.summary}
                </p>
              </div>
            )}
          </motion.div>
        ) : null}
        {openWorldEnabled && isOpenWorldLoading ? (
          <p className="mt-12 text-sm text-[#5f6368]">正在恢复开放世界…</p>
        ) : null}
        {openWorldEnabled && isOpenWorldError ? (
          <div className="mt-12 flex items-center justify-between gap-4 rounded-xl border border-[#f2b8b5] bg-[#fce8e6] p-4 text-[#b3261e]" role="alert">
            <span className="text-sm">开放世界加载失败，已暂停新的行动。</span>
            <button className="text-sm underline" onClick={() => refetchOpenWorld()}>重试</button>
          </div>
        ) : null}
        {openWorldEnabled && !isOpenWorldLoading && !isOpenWorldError && !openWorld ? (
          <section
            className="surface-card relative mt-14 overflow-hidden bg-[#f8faff] px-6 py-8 md:px-10 md:py-10"
            aria-labelledby="enter-world-title"
          >
            <div className="relative">
              <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.24em] text-[#0b57d0]">
                <Sparkles size={14} aria-hidden="true" /> 新的故事线
              </div>
              <h2
                id="enter-world-title"
                className="mt-4 max-w-xl text-2xl font-semibold leading-tight md:text-3xl"
                style={{ color: '#1f1f1f' }}
              >
                以 {playerEntry?.player?.name} 之名，踏入这个世界
              </h2>
              <p className="mt-4 max-w-xl text-sm leading-7 text-[#5f6368]">
                从这一刻起，原著不再是唯一答案。故事角色仍会追逐各自的目标，而你的每次行动，都将写进这条只属于你的时间线。
              </p>
              <div className="mt-6 flex flex-wrap gap-2 text-xs text-[#3c4043]">
                <span className="rounded-full border border-[#d2e3fc] bg-[#e8f0fe] px-3 py-1.5">
                  入场 · 第 {playerEntry?.player?.canonical_checkpoint_chapter} 章
                </span>
                {entryLocation ? (
                  <span className="flex items-center gap-1.5 rounded-full border border-[#d2e3fc] bg-white px-3 py-1.5">
                    <MapPin size={12} aria-hidden="true" /> {entryLocation.name}
                  </span>
                ) : null}
              </div>
              {startOpenWorld.isError ? (
                <p role="alert" className="mt-4 text-sm text-[#b3261e]">
                  {getApiErrorMessage(startOpenWorld.error, '进入开放世界失败')}
                </p>
              ) : null}
              <button
                className="primary-action mt-7 w-full md:w-auto"
                disabled={startOpenWorld.isPending}
                onClick={() => startOpenWorld.mutate()}
              >
                {startOpenWorld.isPending ? '正在创建时间线…' : '进入开放世界'}
                {!startOpenWorld.isPending ? <ChevronRight size={16} aria-hidden="true" /> : null}
              </button>
            </div>
          </section>
        ) : null}
        {openWorld ? <WorldDashboard novelId={novelId || ''} view={openWorld} /> : null}
      </div>

      {/* 底部翻页导航 */}
      <div className="fixed bottom-0 left-0 right-0 z-20 flex items-center justify-between gap-2 border-t border-[#e1e3e8] bg-white/95 px-3 py-3 shadow-[0_-1px_3px_rgba(60,64,67,0.08)] backdrop-blur-xl sm:px-6 sm:py-4">
        <button
          onClick={goBack}
          disabled={isProgressSaving || (!openWorld && currentChapter <= 1)}
          className="tonal-action shrink-0 px-3 text-sm sm:px-5"
        >
          <ChevronLeft size={14} />
          {openWorld ? '回看行动日志' : '上一章'}
        </button>

        {/* 进度条 */}
        <div className="mx-4 hidden flex-1 sm:block">
          <div className="mb-1 text-center text-[11px] text-[#5f6368]">
            {isPlayerTimeline ? '原著坐标 · ' : ''}{currentChapter} / {novel?.total_chapters || '?'}
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
          disabled={isProgressSaving || playerEntityRequired || branchChoiceRequired || !novel || (!openWorld && currentChapter >= novel.total_chapters)}
          className="tonal-action min-w-0 px-3 text-sm sm:px-5"
        >
          {playerEntityRequired
            ? '请先创建角色'
            : branchChoiceRequired
              ? '请先选择'
              : isPlayerTimeline
                ? '继续旅程'
                : '下一章'}
          <ChevronRight size={14} />
        </button>
      </div>

      {/* 角色列表侧边栏 */}
      <AnimatePresence>
        {showCharacterList && (
          <motion.div
            initial={{ opacity: 0, x: 300 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: 300 }}
            transition={{ duration: 0.25, ease: [0.23, 1, 0.32, 1] }}
            className="fixed bottom-0 right-0 top-14 z-30 w-[min(280px,100vw)] space-y-3 overflow-y-auto border-l border-[#e1e3e8] bg-white/95 p-4 shadow-[-8px_0_28px_rgba(60,64,67,0.1)] backdrop-blur-xl"
            style={{
              backdropFilter: 'blur(20px)',
            }}
          >
            <div className="mb-4 text-xs font-semibold uppercase tracking-widest text-[#0b57d0]">
              故事角色
            </div>
            {characters?.map((char) => {
              const isDead = openWorld?.session.dead_character_ids.includes(char.id) ?? false;
              return (
                <button
                  key={char.id}
                  disabled={!isChatReady || isDead}
                  onClick={() => {
                    if (!isChatReady || isDead) return;
                    setActiveChatCharacter(char);
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
                      {isDead ? '当前时间线已死亡' : char.role === 'protagonist' ? '主角' : char.role === 'antagonist' ? '反派' : '配角'}
                    </div>
                  </div>
                  <MessageCircle size={14} className="ml-auto flex-shrink-0 text-[#0b57d0]" />
                </button>
              );
            })}
          </motion.div>
        )}
      </AnimatePresence>

      {/* 角色对话面板 */}
      {activeChatCharacter && (
        <ChatPanel
          character={activeChatCharacter}
          novelId={novelId!}
          currentChapter={currentChapter}
          readerIdentity={readingProgress?.reader_identity}
          canChat={isChatReady && activeCharacterIsAvailable}
          isOpen={!!activeChatCharacter}
          onClose={() => setActiveChatCharacter(null)}
        />
      )}
    </div>
  );
}
