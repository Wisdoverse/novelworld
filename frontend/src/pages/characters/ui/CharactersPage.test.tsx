import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { CharactersPage } from './CharactersPage';

const mocks = vi.hoisted(() => ({
  characters: [] as Array<Record<string, unknown>>,
}));

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
  useCharacters: () => ({ data: mocks.characters, isLoading: false }),
}));
vi.mock('@/widgets/character-card/ui/CharacterCard', () => ({
  CharacterCard: ({ character, onTalk }: {
    character: { name: string };
    onTalk: (character: unknown) => void;
  }) => <button onClick={() => onTalk(character)}>Talk {character.name}</button>,
}));
vi.mock('@/widgets/chat-panel/ui/ChatPanel', () => ({
  ChatPanel: ({ character }: { character: { name: string } }) => (
    <div data-testid="chat-panel">{character.name}</div>
  ),
}));

describe('CharactersPage progress gate', () => {
  it('closes chat when refreshed progress removes the active character', async () => {
    mocks.characters = [{
      id: 'future',
      novel_id: 'novel',
      name: 'Future',
      aliases: [],
      role: 'supporting',
      avatar_status: 'pending',
      first_appearance_chapter: 2,
    }];
    const view = render(<CharactersPage />);

    fireEvent.click(screen.getByRole('button', { name: 'Talk Future' }));
    expect(screen.getByTestId('chat-panel').textContent).toBe('Future');

    mocks.characters = [];
    view.rerender(<CharactersPage />);
    await waitFor(() => expect(screen.queryByTestId('chat-panel')).toBeNull());
  });
});
