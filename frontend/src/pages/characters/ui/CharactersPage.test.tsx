import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CharactersPage } from './CharactersPage';

const mocks = vi.hoisted(() => ({
  characters: [] as Array<Record<string, unknown>>,
  progressErrorCode: undefined as string | undefined,
  refetchProgress: vi.fn(),
  resetIdentity: vi.fn(),
  resetIdentityPending: false,
}));

vi.mock('react-router-dom', () => ({
  useNavigate: () => vi.fn(),
  useParams: () => ({ novelId: 'novel' }),
}));
vi.mock('@/features/auth', () => ({
  useAuthStore: () => ({ user: { id: 'user' } }),
}));
vi.mock('@/entities/reading-progress', () => ({
  useReadingProgress: () => ({
    data: mocks.progressErrorCode
      ? undefined
      : { current_chapter: 2, reader_identity_type: 'self' },
    isLoading: false,
    isError: Boolean(mocks.progressErrorCode),
    error: mocks.progressErrorCode
      ? {
          isAxiosError: true,
          response: { data: { error: { code: mocks.progressErrorCode } } },
        }
      : null,
    refetch: mocks.refetchProgress,
  }),
  useResetReaderIdentity: () => ({
    mutate: mocks.resetIdentity,
    isPending: mocks.resetIdentityPending,
  }),
}));
vi.mock('@/entities/novel', () => ({
  useCharacters: () => ({ data: mocks.characters, isLoading: false }),
}));
vi.mock('@/widgets/character-card', () => ({
  CharacterCard: ({ character, onTalk }: {
    character: { name: string };
    onTalk: (character: unknown) => void;
  }) => <button onClick={() => onTalk(character)}>Talk {character.name}</button>,
}));
vi.mock('@/widgets/chat-panel', () => ({
  ChatPanel: ({ character }: {
    character: { name: string; role?: string; avatar_url?: string };
  }) => (
    <div data-testid="chat-panel">
      {character.name}|{character.role ?? '角色'}|{character.avatar_url ?? 'no-avatar'}
    </div>
  ),
}));

describe('CharactersPage progress gate', () => {
  beforeEach(() => {
    mocks.characters = [];
    mocks.progressErrorCode = undefined;
    mocks.resetIdentityPending = false;
    mocks.refetchProgress.mockReset();
    mocks.resetIdentity.mockReset();
  });

  it('offers the explicit self-identity recovery for an unavailable reader identity', () => {
    mocks.progressErrorCode = 'reader_identity_unavailable';
    render(<CharactersPage />);

    fireEvent.click(screen.getByRole('button', { name: '以本人身份继续' }));

    expect(mocks.resetIdentity).toHaveBeenCalledOnce();
    expect(screen.queryByRole('button', { name: '重试' })).toBeNull();
  });

  it('keeps ordinary progress failures on the existing retry path', () => {
    mocks.progressErrorCode = 'progress_unavailable';
    render(<CharactersPage />);

    fireEvent.click(screen.getByRole('button', { name: '重试' }));

    expect(mocks.refetchProgress).toHaveBeenCalledOnce();
    expect(mocks.resetIdentity).not.toHaveBeenCalled();
  });

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
    expect(screen.getByTestId('chat-panel').textContent).toContain('Future');

    mocks.characters = [];
    view.rerender(<CharactersPage />);
    await waitFor(() => expect(screen.queryByTestId('chat-panel')).toBeNull());
  });

  it('replaces a selected full persona with the latest partial view for the same id', () => {
    mocks.characters = [{
      id: 'same',
      novel_id: 'novel',
      name: 'Same',
      aliases: ['Future Alias'],
      role: 'protagonist',
      avatar_url: 'future-avatar',
      avatar_status: 'ready',
      first_appearance_chapter: 1,
      persona_source_chapter_high_water: 2,
    }];
    const view = render(<CharactersPage />);

    fireEvent.click(screen.getByRole('button', { name: 'Talk Same' }));
    expect(screen.getByTestId('chat-panel').textContent).toContain('future-avatar');

    mocks.characters = [{
      id: 'same',
      novel_id: 'novel',
      name: 'Same',
      first_appearance_chapter: 1,
    }];
    view.rerender(<CharactersPage />);

    expect(screen.getByTestId('chat-panel').textContent).toBe('Same|角色|no-avatar');
  });
});
