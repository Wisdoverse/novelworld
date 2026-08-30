import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CharactersPage } from './CharactersPage';

const mocks = vi.hoisted(() => ({
  characters: [] as Array<Record<string, unknown>>,
  charactersError: false,
  charactersCachedOnError: false,
  refetchCharacters: vi.fn(),
  progressErrorCode: undefined as string | undefined,
  progressCachedOnError: false,
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
    data: mocks.progressErrorCode && !mocks.progressCachedOnError
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
  useCharacters: () => ({
    data: mocks.charactersError && !mocks.charactersCachedOnError
      ? undefined
      : mocks.characters,
    isLoading: false,
    isError: mocks.charactersError,
    refetch: mocks.refetchCharacters,
  }),
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
    mocks.charactersError = false;
    mocks.charactersCachedOnError = false;
    mocks.progressErrorCode = undefined;
    mocks.progressCachedOnError = false;
    mocks.resetIdentityPending = false;
    mocks.refetchProgress.mockReset();
    mocks.resetIdentity.mockReset();
    mocks.refetchCharacters.mockReset();
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

  it('distinguishes a character query failure from an empty cast and offers retry', () => {
    mocks.charactersError = true;
    render(<CharactersPage />);

    expect(screen.getByRole('heading', { name: '暂时无法加载角色' })).toBeTruthy();
    expect(screen.queryByRole('heading', { name: '暂时还没有角色' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: '重试' }));
    expect(mocks.refetchCharacters).toHaveBeenCalledOnce();
  });

  it('keeps cached characters visible when background refreshes fail', () => {
    mocks.progressErrorCode = 'progress_unavailable';
    mocks.progressCachedOnError = true;
    mocks.charactersError = true;
    mocks.charactersCachedOnError = true;
    mocks.characters = [{
      id: 'cached',
      novel_id: 'novel',
      name: 'Cached',
      first_appearance_chapter: 1,
    }];
    render(<CharactersPage />);

    expect(screen.getByRole('button', { name: 'Talk Cached' })).toBeTruthy();
    expect(screen.queryByRole('heading', { name: '暂时无法加载角色' })).toBeNull();
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
