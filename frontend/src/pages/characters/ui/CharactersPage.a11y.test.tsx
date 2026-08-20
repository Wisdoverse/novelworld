import React from 'react';
import { render } from '@testing-library/react';
import { describe, it, vi } from 'vitest';
import { expectNoA11yViolations } from '@/a11y';
import { CharactersPage } from './CharactersPage';

vi.mock('react-router-dom', () => ({
  useNavigate: () => vi.fn(),
  useParams: () => ({ novelId: 'novel' }),
}));
vi.mock('@/features/auth/model/useAuthStore', () => ({
  useAuthStore: () => ({ user: { id: 'user' } }),
}));
vi.mock('@/entities/reading-progress/api', () => ({
  useReadingProgress: () => ({
    data: { current_chapter: 2, reader_identity_type: 'self' },
    isLoading: false,
    isError: false,
    refetch: vi.fn(),
  }),
}));
vi.mock('@/entities/novel/api', () => ({
  useCharacters: () => ({ data: [], isLoading: false }),
}));
vi.mock('@/widgets/character-card/ui/CharacterCard', () => ({
  CharacterCard: ({ character }: { character: { name: string } }) => (
    <button>Talk {character.name}</button>
  ),
}));
vi.mock('@/widgets/chat-panel/ui/ChatPanel', () => ({
  ChatPanel: ({ character }: { character: { name: string } }) => (
    <div>{character.name}</div>
  ),
}));

describe('CharactersPage a11y', () => {
  it('has no axe violations', async () => {
    const { container } = render(<CharactersPage />);
    await expectNoA11yViolations(container);
  });
});