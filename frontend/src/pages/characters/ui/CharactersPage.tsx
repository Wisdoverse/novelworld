import React, { useState } from 'react';
import { useParams, useNavigate } from 'react-router-dom';
import { useCharacters } from '@/entities/novel/api';
import { CharacterCard } from '@/widgets/character-card/ui/CharacterCard';
import { ChatPanel } from '@/widgets/chat-panel/ui/ChatPanel';
import { useAuthStore } from '@/features/auth/model/useAuthStore';
import type { Character } from '@/shared/types';
import { ArrowLeft } from 'lucide-react';

export function CharactersPage() {
  const { novelId } = useParams<{ novelId: string }>();
  const navigate = useNavigate();
  const { user } = useAuthStore();
  const { data: characters, isLoading } = useCharacters(novelId || '');
  const [chatCharacter, setChatCharacter] = useState<Character | null>(null);

  if (!novelId || !user) return null;

  return (
    <div className="min-h-screen px-6 py-8"
         style={{ background: 'linear-gradient(135deg, var(--color-void) 0%, var(--color-cosmos) 100%)' }}>
      <div className="max-w-6xl mx-auto">
        <div className="flex items-center gap-4 mb-8">
          <button
            onClick={() => navigate(-1)}
            className="p-2 rounded-lg transition-all"
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

        {isLoading ? (
          <div className="text-center py-20" style={{ color: 'var(--color-moonbeam)' }}>加载中...</div>
        ) : !characters?.length ? (
          <div className="text-center py-20" style={{ color: 'var(--color-comet)' }}>暂无角色</div>
        ) : (
          <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 gap-4">
            {characters.map((char) => (
              <CharacterCard
                key={char.id}
                character={char}
                onTalk={(c) => setChatCharacter(c)}
              />
            ))}
          </div>
        )}
      </div>

      {chatCharacter && (
        <ChatPanel
          character={chatCharacter}
          userId={user.id}
          novelId={novelId}
          currentChapter={1}
          isOpen={!!chatCharacter}
          onClose={() => setChatCharacter(null)}
        />
      )}
    </div>
  );
}
