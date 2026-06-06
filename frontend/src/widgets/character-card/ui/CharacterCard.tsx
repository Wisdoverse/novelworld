import React from 'react';
import type { Character } from '@/shared/types';
import { MessageCircle, User } from 'lucide-react';

interface Props {
  character: Character;
  onTalk: (character: Character) => void;
}

const roleBadgeColors: Record<string, string> = {
  protagonist: '#6d28d9',
  antagonist: '#dc2626',
  supporting: '#0891b2',
  minor: '#475569',
};

const roleLabels: Record<string, string> = {
  protagonist: '主角',
  antagonist: '反派',
  supporting: '配角',
  minor: '路人',
};

export function CharacterCard({ character, onTalk }: Props) {
  return (
    <div
      className="rounded-xl overflow-hidden transition-all hover:scale-[1.02]"
      style={{
        background: 'rgba(15, 21, 53, 0.6)',
        border: '1px solid rgba(109, 40, 217, 0.2)',
      }}
    >
      <div className="aspect-square relative overflow-hidden"
           style={{ background: 'rgba(3, 4, 10, 0.4)' }}>
        {character.avatar_url ? (
          <img
            src={character.avatar_url}
            alt={character.name}
            className="w-full h-full object-cover"
          />
        ) : (
          <div className="w-full h-full flex items-center justify-center">
            <User size={48} style={{ color: 'var(--color-comet)' }} />
          </div>
        )}
        <span
          className="absolute top-2 right-2 px-2 py-0.5 rounded-full text-xs font-medium text-white"
          style={{ background: roleBadgeColors[character.role] || '#475569' }}
        >
          {roleLabels[character.role] || character.role}
        </span>
      </div>

      <div className="p-4">
        <h3 className="font-semibold text-lg mb-1" style={{ color: 'var(--color-starlight)' }}>
          {character.name}
        </h3>
        {character.aliases.length > 0 && (
          <p className="text-xs mb-2" style={{ color: 'var(--color-comet)' }}>
            别名：{character.aliases.join('、')}
          </p>
        )}
        <p className="text-sm line-clamp-2 mb-3" style={{ color: 'var(--color-moonbeam)' }}>
          {character.description || '暂无描述'}
        </p>

        <button
          onClick={() => onTalk(character)}
          className="w-full flex items-center justify-center gap-2 py-2 rounded-lg transition-all"
          style={{
            background: 'rgba(109, 40, 217, 0.2)',
            border: '1px solid rgba(109, 40, 217, 0.3)',
            color: 'var(--color-aurora-light)',
          }}
        >
          <MessageCircle size={16} />
          对话
        </button>
      </div>
    </div>
  );
}
