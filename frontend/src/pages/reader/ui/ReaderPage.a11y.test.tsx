import React from 'react';
import { render } from '@testing-library/react';
import { describe, it, vi } from 'vitest';
import { expectNoA11yViolations } from '@/a11y';
import { ReaderPage } from './ReaderPage';

const mocks = vi.hoisted(() => ({
  navigate: vi.fn(),
  mutate: vi.fn(),
  reset: vi.fn(),
  progressSaving: false,
  progressError: false,
  progressChapter: 2,
  identityType: 'self',
  hasBranch: false,
  player: null as Record<string, unknown> | null,
  playerEntryEnabled: false,
  playerEntryCheckpoint: undefined as number | undefined,
  branchEnabled: false,
  branchNode: undefined,
  createPlayer: vi.fn(),
  startWorld: vi.fn(),
  openWorld: null,
  characters: [] as Array<Record<string, unknown>>,
  effectiveContent: 'Chapter two',
  effectiveGenerated: false,
  effectiveError: false,
}));

vi.mock('react-router-dom', () => ({
  useNavigate: () => mocks.navigate,
  useParams: () => ({ novelId: 'novel', chapterNum: '2' }),
}));
vi.mock('@/entities/novel/api', () => ({
  useNovel: () => ({ data: { id: 'novel', title: 'Novel', total_chapters: 3 } }),
  useChapter: () => ({
    data: { chapter_number: 2, title: 'Two', content: 'Chapter two', is_key_node: false, key_node_description: undefined },
    isLoading: false,
  }),
  useCharacters: () => ({ data: mocks.characters }),
}));
vi.mock('@/entities/reading-progress/api', () => ({
  useReadingProgress: () => ({
    data: { current_chapter: mocks.progressChapter, reader_identity_type: mocks.identityType, deviation_mode: 'canon' },
    isLoading: false, isError: false, refetch: vi.fn(),
  }),
  useUpdateReadingProgress: () => ({ mutate: mocks.mutate, isPending: false, isError: false, reset: mocks.reset }),
}));
vi.mock('@/entities/narrative/api', () => ({
  useEffectiveChapter: () => ({ data: { chapter_number: 2, content: mocks.effectiveContent, generated: mocks.effectiveGenerated }, isLoading: false, isError: false, refetch: vi.fn() }),
  useNarrativeNode: () => ({ data: undefined, isLoading: false, isError: false, refetch: vi.fn() }),
  usePlayerEntry: () => ({ data: { player: null, checkpoint_chapter: 2, locations: [{ id: 'tower', name: '北塔' }] }, isLoading: false, isError: false, refetch: vi.fn() }),
  useCreatePlayerEntity: () => ({ mutateAsync: vi.fn(), isPending: false, isError: false }),
  useWorldState: () => ({ data: undefined }),
  useOpenWorld: () => ({ data: null, isLoading: false, isError: false, refetch: vi.fn() }),
  useStartOpenWorld: () => ({ mutate: vi.fn(), isPending: false, isError: false }),
  useSubmitNarrativeChoice: () => ({ mutateAsync: vi.fn(), isPending: false }),
}));
vi.mock('@/widgets/chat-panel/ui/ChatPanel', () => ({
  ChatPanel: ({ character }: { character: { name: string } }) => <div>{character.name}</div>,
}));
vi.mock('@/widgets/branch-choice/ui/BranchChoice', () => ({ BranchChoice: () => <div /> }));
vi.mock('@/widgets/world-dashboard/ui/WorldDashboard', () => ({ WorldDashboard: () => null }));

describe('ReaderPage a11y', () => {
  it('has no axe violations on the settled reader', async () => {
    const { container } = render(<ReaderPage />);
    await expectNoA11yViolations(container);
  });
});