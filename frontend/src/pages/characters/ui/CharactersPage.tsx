import React, { useEffect, useState } from 'react';
import { useParams, useNavigate } from 'react-router-dom';
import { useCharacters } from '@/entities/novel/api';
import { useReadingProgress } from '@/entities/reading-progress/api';
import { CharacterCard } from '@/widgets/character-card/ui/CharacterCard';
import { ChatPanel } from '@/widgets/chat-panel/ui/ChatPanel';
import { useAuthStore } from '@/features/auth/model/useAuthStore';
import type { Character } from '@/shared/types';
import { ArrowLeft } from 'lucide-react';

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
    <div className="min-h-screen px-6 py-8"
         style={{ background: 'linear-gradient(135deg, var(--color-void) 0%, var(--color-cosmos) 100%)' }}>
      <div className="max-w-6xl mx-auto">
        <div className="flex items-center gap-4 mb-8">
          <button
            onClick={() => navigate(-1)}
            className="p-2 rounded-lg transition-all"
            aria-label="返回"
            style={{
              background: 'rgba(15, 21, 53, 0.6)',
              border: '1px solid rgba(109, 40, 217, 0.2)',
              color: 'var(--color-moonbeam)',
            }}
          >
            <ArrowLeft size={20} />
          </button>
          <h1 className="text-2xl font-bold" style={{ fontFamily: 'var(--font-display)', color: 'var(--color-starlight)' }}>
            角色列表
          </h1>
        </div>

        {isLoading || isProgressLoading ? (
          <div className="text-center py-20" style={{ color: 'var(--color-moonbeam)' }}>加载中...</div>
        ) : isProgressError ? (
          <div className="text-center py-20" role="alert" style={{ color: 'var(--color-moonbeam)' }}>
            <p className="mb-4">阅读上下文加载失败，暂时无法开始对话。</p>
            <button className="px-4 py-2 rounded-lg" style={{ background: '#6d28d9', color: 'white' }} onClick={() => refetchProgress()}>
              重试
            </button>
          </div>
        ) : !characters?.length ? (
          <div className="text-center py-20" style={{ color: 'var(--color-comet)' }}>暂无角色</div>
        ) : (
          <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 gap-4">
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
