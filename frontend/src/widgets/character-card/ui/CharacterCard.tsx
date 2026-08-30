import type { Character } from '@/shared/types';
import { MessageCircle, User } from 'lucide-react';

interface Props {
  character: Character;
  onTalk: (character: Character) => void;
}

const roleBadgeColors: Record<string, string> = {
  protagonist: '#0b57d0',
  antagonist: '#b3261e',
  supporting: '#188038',
  minor: '#5f6368',
};

const roleLabels: Record<string, string> = {
  protagonist: '主角',
  antagonist: '反派',
  supporting: '配角',
  minor: '路人',
};

export function CharacterCard({ character, onTalk }: Props) {
  const role = character.role;

  return (
    <div
      className="surface-card overflow-hidden transition-transform hover:-translate-y-1"
    >
      <div className="relative aspect-square overflow-hidden bg-[#eef3ff]">
        {character.avatar_url ? (
          <img
            src={character.avatar_url}
            alt={character.name}
            className="w-full h-full object-cover"
          />
        ) : (
          <div className="w-full h-full flex items-center justify-center">
            <User size={48} style={{ color: '#7b8db7' }} />
          </div>
        )}
        {role ? (
          <span
            className="absolute top-2 right-2 px-2 py-0.5 rounded-full text-xs font-medium text-white"
            style={{ background: roleBadgeColors[role] || '#475569' }}
          >
            {roleLabels[role] || role}
          </span>
        ) : null}
      </div>

      <div className="p-4">
        <h3 className="font-semibold text-lg mb-1 text-[#1f1f1f]">
          {character.name}
        </h3>
        {character.aliases?.length ? (
          <p className="text-xs mb-2 text-[#5f6368]">
            别名：{character.aliases.join('、')}
          </p>
        ) : null}
        <p className="text-sm line-clamp-2 mb-3 text-[#5f6368]">
          {character.description || '暂无描述'}
        </p>

        <button
          onClick={() => onTalk(character)}
          className="tonal-action w-full text-sm"
        >
          <MessageCircle size={16} />
          对话
        </button>
      </div>
    </div>
  );
}
