import React, { useState, useEffect, useRef } from 'react';
import { useParams, useNavigate } from 'react-router-dom';
import { motion, AnimatePresence } from 'framer-motion';
import {
  ChevronLeft, ChevronRight, MessageCircle, Users,
  BookOpen, Sparkles
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
  type ChoiceResult,
} from '@/entities/narrative/api';
import { ChatPanel } from '@/widgets/chat-panel/ui/ChatPanel';
import { BranchChoice } from '@/widgets/branch-choice/ui/BranchChoice';
import { WorldDashboard } from '@/widgets/world-dashboard/ui/WorldDashboard';
import { PlayerEntryForm } from '@/features/player-entry/ui/PlayerEntryForm';
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

  const [activeChatCharacter, setActiveChatCharacter] = useState<Character | null>(null);
  const [showCharacterList, setShowCharacterList] = useState(false);
  const [choiceResult, setChoiceResult] = useState<ChoiceResult | null>(null);
  const [submittedChoiceIndex, setSubmittedChoiceIndex] = useState<number | undefined>();
  const [choiceError, setChoiceError] = useState<string | undefined>();
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
  const { data: worldState } = useWorldState(novelId || '', Boolean(chapter));
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
    setSubmittedChoiceIndex(undefined);
    setChoiceError(undefined);
  }, [currentChapter, novelId]);

  useEffect(() => {
    setEntryCheckpoint(undefined);
  }, [novelId]);

  const savedChoice = currentBranchNode
    ? worldState?.state.choices.find(choice => choice.node_id === currentBranchNode.id)
    : undefined;
  const selectedChoiceIndex = submittedChoiceIndex ?? savedChoice?.choice_index;
  const consequence = choiceResult?.consequence ?? savedChoice?.consequence;
  const displayContent = effectiveChapter?.content ?? '';
  const inlineChapter = chapter && currentBranchNode
    ? splitChapterAtAnchor(displayContent, currentBranchNode.anchor_quote)
    : undefined;
  const branchChoiceRequired = Boolean(
    currentBranchNode && selectedChoiceIndex === undefined && !openWorld,
  );

  const handleChoose = async (choice: NarrativeChoice) => {
    if (!currentBranchNode || openWorld) return;
    setChoiceError(undefined);
    try {
      const result = await submitChoice.mutateAsync({
        nodeId: currentBranchNode.id,
        choiceIndex: choice.index,
      });
      setSubmittedChoiceIndex(choice.index);
      setChoiceResult(result);
    } catch (error) {
      setChoiceError(getApiErrorMessage(error, '命运改写失败'));
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

  if (isProgressError) {
    return (
      <div className="min-h-screen flex flex-col gap-4 items-center justify-center" style={{ background: 'var(--color-void)', color: '#94a3b8' }}>
        <p role="alert">阅读进度加载失败，无法安全恢复阅读上下文。</p>
        <div className="flex gap-3">
          <button className="px-4 py-2 rounded-lg" style={{ background: '#6d28d9', color: 'white' }} onClick={() => refetchProgress()}>
            重试
          </button>
          <button className="px-4 py-2 rounded-lg" style={{ background: 'rgba(255,255,255,0.08)' }} onClick={() => navigate('/shelf')}>
            返回书架
          </button>
        </div>
      </div>
    );
  }

  if (routeChapter === undefined || currentChapter < 1 || isProgressLoading) {
    return (
      <div className="min-h-screen flex items-center justify-center" style={{ background: 'var(--color-void)' }}>
        <div className="w-8 h-8 border-2 rounded-full animate-spin" style={{ borderColor: '#6d28d9', borderTopColor: 'transparent' }} />
      </div>
    );
  }

  return (
    <div className="min-h-screen" style={{ background: 'var(--color-void)' }}>
      {/* 顶部导航栏 */}
      <motion.header
        initial={{ y: -60 }}
        animate={{ y: 0 }}
        className="fixed top-0 left-0 right-0 z-40 flex items-center justify-between px-6 py-3"
        style={{
          background: 'rgba(3, 4, 10, 0.9)',
          backdropFilter: 'blur(20px)',
          borderBottom: '1px solid rgba(109, 40, 217, 0.15)',
        }}
      >
        <div className="flex items-center gap-4">
          <button
            onClick={() => navigate('/shelf')}
            className="flex items-center gap-2 text-sm transition-colors"
            style={{ color: '#94a3b8' }}
          >
            <ChevronLeft size={16} />
            书架
          </button>
          <div className="w-px h-4" style={{ background: 'rgba(255,255,255,0.1)' }} />
          <div>
            <div className="text-sm font-medium" style={{ color: '#e2e8f0' }}>
              {novel?.title}
            </div>
            <div className="text-xs" style={{ color: '#475569' }}>
              {chapter?.title || `第 ${currentChapter} 章`}
            </div>
          </div>
        </div>

        <div className="flex items-center gap-2">
          {/* 进度 */}
          <div className="hidden md:flex items-center gap-2 text-xs" style={{ color: '#475569' }}>
            <BookOpen size={12} />
            {currentChapter} / {novel?.total_chapters || '?'}
          </div>

          {/* 角色列表按钮 */}
          <button
            onClick={() => setShowCharacterList(!showCharacterList)}
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs transition-all"
            style={{
              background: showCharacterList ? 'rgba(6, 182, 212, 0.15)' : 'rgba(255,255,255,0.05)',
              border: '1px solid rgba(6, 182, 212, 0.2)',
              color: '#22d3ee',
            }}
          >
            <Users size={12} />
            角色
          </button>
        </div>
      </motion.header>

      {/* 主内容区 */}
      <div className="pt-16 pb-24 px-4 md:px-8 max-w-3xl mx-auto">
        {isProgressSaveError && (
          <div className="mt-4 p-3 rounded-lg flex items-center justify-between gap-3" role="alert" style={{ background: 'rgba(220, 38, 38, 0.12)', color: '#fca5a5' }}>
            <span className="text-sm">阅读进度保存失败，聊天已暂停。</span>
            <button className="text-sm underline" onClick={retryProgressUpdate}>重试</button>
          </div>
        )}
        {isSelfMode && isPlayerEntryLoading ? (
          <p className="mt-8 text-sm" style={{ color: '#94a3b8' }}>正在恢复你的原创角色…</p>
        ) : null}
        {isSelfMode && isPlayerEntryError ? (
          <div className="mt-8 p-4 rounded-xl flex items-center justify-between gap-4" role="alert" style={{ background: 'rgba(220, 38, 38, 0.1)', color: '#fca5a5' }}>
            <span className="text-sm">原创角色加载失败，命运分支已暂停。</span>
            <button className="text-sm underline" onClick={() => refetchPlayerEntry()}>重试</button>
          </div>
        ) : null}
        {isSelfMode && playerEntry && !playerEntry.player ? (
          <PlayerEntryForm
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
            <div className="w-8 h-8 border-2 rounded-full animate-spin" style={{ borderColor: '#6d28d9', borderTopColor: 'transparent' }} />
          </div>
        ) : isEffectiveChapterError ? (
          <div
            role="alert"
            className="mt-16 p-5 rounded-xl flex items-center justify-between gap-4"
            style={{ background: 'rgba(220, 38, 38, 0.1)', border: '1px solid rgba(248, 113, 113, 0.25)', color: '#fca5a5' }}
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
          >
            {/* 章节标题 */}
            <div className="text-center mb-12 pt-8">
              <div className="text-xs font-semibold uppercase tracking-widest mb-2" style={{ color: '#6d28d9' }}>
                第 {currentChapter} 章
              </div>
              {effectiveChapter.generated && (
                <div className="mb-3 text-xs font-semibold tracking-wider" style={{ color: '#22d3ee' }}>
                  玩家时间线 · 本章已因你的选择完全改写
                </div>
              )}
              {chapter.title && (
                <h1
                  className="text-2xl md:text-3xl font-bold"
                  style={{ fontFamily: 'var(--font-display)', color: '#e2e8f0' }}
                >
                  {chapter.title}
                </h1>
              )}
              <div className="mt-4 mx-auto w-16 h-px" style={{ background: 'linear-gradient(90deg, transparent, #6d28d9, transparent)' }} />
            </div>

            {/* 正文中的分支节点：原文在锚点处暂停，选择后由生成内容接续。 */}
            {hasBranch && isBranchLoading && (
              <div className="my-16 flex items-center justify-center gap-2 p-5 text-sm" style={{ color: '#22d3ee' }}>
                <div className="w-4 h-4 border-2 rounded-full animate-spin" style={{ borderColor: '#22d3ee', borderTopColor: 'transparent' }} />
                正在定位章节中的命运交叉点...
              </div>
            )}
            {(!hasBranch || !isBranchLoading) && (
              <div className="reader-content">
                {(inlineChapter?.before ?? displayContent).split('\n\n').map((paragraph, i) => (
                  <p key={i}>{paragraph}</p>
                ))}
              </div>
            )}
            {hasBranch && isBranchError && (
              <div
                role="alert"
                className="my-8 p-4 rounded-xl flex items-center justify-between gap-4"
                style={{ background: 'rgba(220, 38, 38, 0.1)', border: '1px solid rgba(248, 113, 113, 0.25)', color: '#fca5a5' }}
              >
                <span className="text-sm">命运交叉点加载失败。</span>
                <button className="text-sm underline" onClick={() => refetchBranch()}>重试</button>
              </div>
            )}
            {currentBranchNode && (!openWorld || selectedChoiceIndex !== undefined) && (
              <BranchChoice
                node={currentBranchNode}
                onChoose={handleChoose}
                isLoading={submitChoice.isPending}
                selectedChoiceIndex={selectedChoiceIndex}
                consequence={consequence}
                error={choiceError}
              />
            )}

            {/* 章节摘要 */}
            {chapter.summary && !hasBranch && (
              <div
                className="mt-12 p-4 rounded-xl"
                style={{
                  background: 'rgba(109, 40, 217, 0.08)',
                  border: '1px solid rgba(109, 40, 217, 0.2)',
                }}
              >
                <div className="flex items-center gap-2 mb-2 text-xs font-semibold uppercase tracking-wider" style={{ color: '#8b5cf6' }}>
                  <Sparkles size={12} />
                  章节摘要
                </div>
                <p className="text-sm leading-relaxed" style={{ color: '#94a3b8' }}>
                  {chapter.summary}
                </p>
              </div>
            )}
          </motion.div>
        ) : null}
        {openWorldEnabled && isOpenWorldLoading ? (
          <p className="mt-12 text-sm" style={{ color: '#94a3b8' }}>正在恢复开放世界…</p>
        ) : null}
        {openWorldEnabled && isOpenWorldError ? (
          <div className="mt-12 p-4 rounded-xl flex items-center justify-between gap-4" role="alert" style={{ background: 'rgba(220, 38, 38, 0.1)', color: '#fca5a5' }}>
            <span className="text-sm">开放世界加载失败，已暂停新的行动。</span>
            <button className="text-sm underline" onClick={() => refetchOpenWorld()}>重试</button>
          </div>
        ) : null}
        {openWorldEnabled && !isOpenWorldLoading && !isOpenWorldError && !openWorld ? (
          <section className="mt-12 p-6 rounded-2xl" style={{ background: 'rgba(6, 182, 212, 0.07)', border: '1px solid rgba(6, 182, 212, 0.25)' }} aria-labelledby="enter-world-title">
            <h2 id="enter-world-title" className="text-xl font-semibold" style={{ color: '#e2e8f0' }}>进入小说的开放世界</h2>
            <p className="mt-2 text-sm leading-6" style={{ color: '#94a3b8' }}>
              从角色创建时的第 {playerEntry?.player?.canonical_checkpoint_chapter} 章进入；原著角色保有自己的目标，你只决定自己的行动。
            </p>
            {startOpenWorld.isError ? (
              <p role="alert" className="mt-3 text-sm" style={{ color: '#fca5a5' }}>
                {getApiErrorMessage(startOpenWorld.error, '进入开放世界失败')}
              </p>
            ) : null}
            <button
              className="mt-4 px-4 py-2 rounded-lg text-sm font-medium disabled:opacity-50"
              style={{ background: '#0891b2', color: 'white' }}
              disabled={startOpenWorld.isPending}
              onClick={() => startOpenWorld.mutate()}
            >
              {startOpenWorld.isPending ? '正在创建时间线…' : '进入开放世界'}
            </button>
          </section>
        ) : null}
        {openWorld ? <WorldDashboard novelId={novelId || ''} view={openWorld} /> : null}
      </div>

      {/* 底部翻页导航 */}
      <div
        className="fixed bottom-0 left-0 right-0 flex items-center justify-between px-6 py-4"
        style={{
          background: 'rgba(3, 4, 10, 0.95)',
          backdropFilter: 'blur(20px)',
          borderTop: '1px solid rgba(109, 40, 217, 0.15)',
        }}
      >
        <button
          onClick={() => goToChapter(currentChapter - 1)}
          disabled={isProgressSaving || currentChapter <= 1}
          className="flex items-center gap-2 px-4 py-2 rounded-lg text-sm transition-all"
          style={{
            background: 'rgba(255,255,255,0.05)',
            border: '1px solid rgba(255,255,255,0.1)',
            color: isProgressSaving || currentChapter <= 1 ? '#334155' : '#94a3b8',
            cursor: isProgressSaving || currentChapter <= 1 ? 'not-allowed' : 'pointer',
          }}
        >
          <ChevronLeft size={14} />
          上一章
        </button>

        {/* 进度条 */}
        <div className="flex-1 mx-6">
          <div className="progress-cosmic">
            <div
              className="progress-cosmic-fill"
              style={{ width: `${novel ? (currentChapter / novel.total_chapters) * 100 : 0}%` }}
            />
          </div>
        </div>

        <button
          onClick={() => goToChapter(currentChapter + 1)}
          disabled={isProgressSaving || playerEntityRequired || branchChoiceRequired || !novel || currentChapter >= novel.total_chapters}
          className="flex items-center gap-2 px-4 py-2 rounded-lg text-sm transition-all"
          style={{
            background: 'rgba(255,255,255,0.05)',
            border: '1px solid rgba(255,255,255,0.1)',
            color: (isProgressSaving || playerEntityRequired || branchChoiceRequired || !novel || currentChapter >= novel.total_chapters) ? '#334155' : '#94a3b8',
            cursor: (isProgressSaving || playerEntityRequired || branchChoiceRequired || !novel || currentChapter >= novel.total_chapters) ? 'not-allowed' : 'pointer',
          }}
        >
          {playerEntityRequired ? '请先创建角色' : branchChoiceRequired ? '请先选择' : '下一章'}
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
            className="fixed right-0 top-14 bottom-0 z-30 overflow-y-auto p-4 space-y-3"
            style={{
              width: '280px',
              background: 'rgba(8, 13, 31, 0.95)',
              backdropFilter: 'blur(20px)',
              borderLeft: '1px solid rgba(109, 40, 217, 0.2)',
            }}
          >
            <div className="text-xs font-semibold uppercase tracking-widest mb-4" style={{ color: '#6d28d9' }}>
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
                  className="w-full flex items-center gap-3 p-3 rounded-xl text-left transition-all disabled:opacity-50"
                  style={{
                    background: 'rgba(15, 21, 53, 0.6)',
                    border: '1px solid rgba(109, 40, 217, 0.2)',
                  }}
                >
                  {char.avatar_url ? (
                    <img src={char.avatar_url} alt={char.name}
                      className="w-10 h-10 rounded-full object-cover flex-shrink-0"
                      style={{ border: '2px solid rgba(6, 182, 212, 0.3)' }}
                    />
                  ) : (
                    <div className="w-10 h-10 rounded-full flex-shrink-0 flex items-center justify-center font-bold"
                      style={{ background: 'linear-gradient(135deg, #6d28d9, #06b6d4)', color: 'white' }}>
                      {char.name[0]}
                    </div>
                  )}
                  <div className="min-w-0">
                    <div className="text-sm font-medium truncate" style={{ color: '#e2e8f0' }}>{char.name}</div>
                    <div className="text-xs truncate" style={{ color: '#475569' }}>
                      {isDead ? '当前时间线已死亡' : char.role === 'protagonist' ? '主角' : char.role === 'antagonist' ? '反派' : '配角'}
                    </div>
                  </div>
                  <MessageCircle size={14} className="flex-shrink-0 ml-auto" style={{ color: '#22d3ee' }} />
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
