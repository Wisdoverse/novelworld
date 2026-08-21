import React, { useEffect, useState } from 'react';
import { useParams, useNavigate } from 'react-router-dom';
import { useCharacters } from '@/entities/novel/api';
import { useReadingProgress } from '@/entities/reading-progress/api';
import { CharacterCard } from '@/widgets/character-card/ui/CharacterCard';
import { ChatPanel } from '@/widgets/chat-panel/ui/ChatPanel';
import { useAuthStore } from '@/features/auth/model/useAuthStore';
import type { Character } from '@/shared/types';
import { AlertCircle, ArrowLeft, Users } from 'lucide-react';

export function CharactersPage() {
  const { novelId } = useParams<{ novelId: string }>();
  const navigate = useNavigate();
  const { user } = useAuthStore();
  const {
    data: readingProgress,
    isLoading: isProgressLoading,
    isError: isProgressError,
    refetch: refetchProgress,
  } = useReadingProgress(novelId || '');
  const { data: characters, isLoading } = useCharacters(
    novelId || '',
    readingProgress?.current_chapter ?? 0,
  );
  const [chatCharacter, setChatCharacter] = useState<Character | null>(null);
  const chatCharacterIsAvailable = Boolean(
    chatCharacter && characters?.some(character => character.id === chatCharacter.id),
  );

  useEffect(() => {
    if (chatCharacter && characters && !chatCharacterIsAvailable) {
      setChatCharacter(null);
    }
  }, [characters, chatCharacter, chatCharacterIsAvailable]);

  if (!novelId || !user) return null;

  return (
    <div className="app-surface min-h-screen px-4 py-8 sm:px-6 sm:py-10">
      <div className="max-w-6xl mx-auto">
        <div className="mb-8 flex items-center gap-4">
          <button
            onClick={() => navigate(-1)}
            className="flex h-10 w-10 items-center justify-center rounded-full text-[#0b57d0] transition-colors hover:bg-[#e8f0fe]"
            aria-label="返回"
          >
            <ArrowLeft size={20} />
          </button>
          <div>
            <p className="text-sm font-medium text-[#0b57d0]">人物关系</p>
            <h1 className="mt-1 text-3xl font-medium tracking-[-0.02em] text-[#1f1f1f]">角色列表</h1>
          </div>
        </div>

        {isLoading || isProgressLoading ? (
          <div className="surface-card flex items-center justify-center gap-3 py-20 text-sm text-[#5f6368]">
            <div className="h-5 w-5 animate-spin rounded-full border-2 border-[#0b57d0] border-t-transparent" />
            正在加载角色…
          </div>
        ) : isProgressError ? (
          <div className="surface-card px-6 py-16 text-center" role="alert">
            <span className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-[#fce8e6] text-[#b3261e]">
              <AlertCircle size={24} aria-hidden="true" />
            </span>
            <h2 className="mt-5 text-xl font-semibold text-[#1f1f1f]">暂时无法加载角色</h2>
            <p className="mx-auto mt-2 max-w-md text-sm leading-6 text-[#5f6368]">阅读上下文没有成功恢复。你的数据不会丢失，可以稍后重试。</p>
            <button className="primary-action mt-6" onClick={() => refetchProgress()}>重试</button>
          </div>
        ) : !characters?.length ? (
          <div className="surface-card px-6 py-16 text-center">
            <Users size={40} className="mx-auto text-[#7b8db7]" aria-hidden="true" />
            <h2 className="mt-4 text-lg font-semibold text-[#1f1f1f]">暂时还没有角色</h2>
            <p className="mt-2 text-sm text-[#5f6368]">小说解析完成后，角色会显示在这里。</p>
          </div>
        ) : (
          <div className="grid grid-cols-1 gap-5 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-4">
            {characters.map((char) => (
              <CharacterCard
                key={char.id}
                character={char}
                onTalk={(c) => {
                  if (readingProgress) setChatCharacter(c);
                }}
              />
            ))}
          </div>
        )}
      </div>

      {chatCharacter && (
        <ChatPanel
          character={chatCharacter}
          novelId={novelId}
          currentChapter={readingProgress?.current_chapter || 1}
          readerIdentity={readingProgress?.reader_identity}
          canChat={Boolean(readingProgress) && chatCharacterIsAvailable}
          isOpen={!!chatCharacter}
          onClose={() => setChatCharacter(null)}
        />
      )}
    </div>
  );
}
