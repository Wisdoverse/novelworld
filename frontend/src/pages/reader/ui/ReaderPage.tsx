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
import { ChatPanel } from '@/widgets/chat-panel/ui/ChatPanel';
import { BranchChoice } from '@/widgets/branch-choice/ui/BranchChoice';
import type { Character, NarrativeChoice, NarrativeNode } from '@/shared/types';

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
  const { data: characters } = useCharacters(
    novelId || '',
    readingProgress?.current_chapter ?? 0,
  );

  const [activeChatCharacter, setActiveChatCharacter] = useState<Character | null>(null);
  const [showCharacterList, setShowCharacterList] = useState(false);
  const [currentBranchNode, setCurrentBranchNode] = useState<NarrativeNode | null>(null);
  const lastProgressAttempt = useRef<string | undefined>(undefined);

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
      && characters?.some(character => character.id === activeChatCharacter.id),
  );

  useEffect(() => {
    if (activeChatCharacter && characters && !activeCharacterIsAvailable) {
      setActiveChatCharacter(null);
    }
  }, [activeChatCharacter, activeCharacterIsAvailable, characters]);

  // 章节加载完成后检查是否有分支节点
  useEffect(() => {
    if (chapter?.is_key_node && chapter.key_node_description) {
      // 模拟分支节点（实际从 narrative-service 获取）
      setCurrentBranchNode({
        id: crypto.randomUUID(),
        novel_id: novelId!,
        chapter_number: currentChapter,
        description: chapter.key_node_description,
        choices: [
          { index: 0, text: '勇敢地站出来，直面挑战', hint: '也许会改变一切...' },
          { index: 1, text: '谨慎地观察，等待时机', hint: '智者善于等待' },
          { index: 2, text: '寻求盟友的帮助', hint: '团结就是力量' },
        ],
      });
    }
  }, [chapter]);

  const handleChoose = async (choice: NarrativeChoice) => {
    // TODO: 调用 narrative-service 生成后续剧情
    console.log('Chosen:', choice);
    setCurrentBranchNode(null);
  };

  const goToChapter = (num: number) => {
    if (isProgressSaving || num < 1 || (novel && num > novel.total_chapters)) return;
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
        {isLoading ? (
          <div className="flex items-center justify-center h-64">
            <div className="w-8 h-8 border-2 rounded-full animate-spin" style={{ borderColor: '#6d28d9', borderTopColor: 'transparent' }} />
          </div>
        ) : chapter ? (
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

            {/* 正文 */}
            <div className="reader-content">
              {chapter.content.split('\n\n').map((paragraph, i) => (
                <p key={i}>{paragraph}</p>
              ))}
            </div>

            {/* 分支选择节点 */}
            {currentBranchNode && (
              <BranchChoice
                node={currentBranchNode}
                onChoose={handleChoose}
              />
            )}

            {/* 章节摘要 */}
            {chapter.summary && (
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
          disabled={isProgressSaving || !novel || currentChapter >= novel.total_chapters}
          className="flex items-center gap-2 px-4 py-2 rounded-lg text-sm transition-all"
          style={{
            background: 'rgba(255,255,255,0.05)',
            border: '1px solid rgba(255,255,255,0.1)',
            color: (isProgressSaving || !novel || currentChapter >= novel.total_chapters) ? '#334155' : '#94a3b8',
            cursor: (isProgressSaving || !novel || currentChapter >= novel.total_chapters) ? 'not-allowed' : 'pointer',
          }}
        >
          下一章
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
            {characters?.map((char) => (
              <button
                key={char.id}
                disabled={!isChatReady}
                onClick={() => {
                  if (!isChatReady) return;
                  setActiveChatCharacter(char);
                  setShowCharacterList(false);
                }}
                className="w-full flex items-center gap-3 p-3 rounded-xl text-left transition-all"
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
                    {char.role === 'protagonist' ? '主角' : char.role === 'antagonist' ? '反派' : '配角'}
                  </div>
                </div>
                <MessageCircle size={14} className="flex-shrink-0 ml-auto" style={{ color: '#22d3ee' }} />
              </button>
            ))}
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
