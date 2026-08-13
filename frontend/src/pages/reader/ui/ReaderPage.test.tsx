import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { NarrativeNode, OpenWorldView } from '@/shared/types';
import { ReaderPage, splitChapterAtAnchor } from './ReaderPage';

const mocks = vi.hoisted(() => ({
  navigate: vi.fn(),
  mutate: vi.fn(),
  reset: vi.fn(),
  progressSaving: false,
  progressError: true,
  progressChapter: 1,
  identityType: 'self',
  hasBranch: false,
  player: { id: 'player', name: '云舟' } as Record<string, unknown> | null,
  playerEntryEnabled: false,
  playerEntryCheckpoint: undefined as number | undefined,
  branchEnabled: false,
  branchNode: undefined as NarrativeNode | undefined,
  createPlayer: vi.fn(),
  startWorld: vi.fn(),
  openWorld: null as OpenWorldView | null,
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
    data: {
      chapter_number: 2,
      title: 'Two',
      content: 'Chapter two',
      is_key_node: mocks.hasBranch,
      key_node_description: mocks.hasBranch ? 'A choice' : undefined,
    },
    isLoading: false,
  }),
  useCharacters: () => ({ data: mocks.characters }),
}));

vi.mock('@/entities/reading-progress/api', () => ({
  useReadingProgress: () => ({
    data: {
      current_chapter: mocks.progressChapter,
      reader_identity_type: mocks.identityType,
      deviation_mode: 'canon',
    },
    isLoading: false,
    isError: false,
    refetch: vi.fn(),
  }),
  useUpdateReadingProgress: () => ({
    mutate: mocks.mutate,
    isPending: mocks.progressSaving,
    isError: mocks.progressError,
    reset: mocks.reset,
  }),
}));

vi.mock('@/entities/narrative/api', () => ({
  useEffectiveChapter: () => ({
    data: mocks.effectiveError ? undefined : {
      chapter_number: 2,
      content: mocks.effectiveContent,
      generated: mocks.effectiveGenerated,
    },
    isLoading: false,
    isError: mocks.effectiveError,
    refetch: vi.fn(),
  }),
  useNarrativeNode: (_novelId: string, _chapter: number, enabled: boolean) => {
    mocks.branchEnabled = enabled;
    return {
      data: mocks.branchNode,
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    };
  },
  usePlayerEntry: (_novelId: string, enabled: boolean, checkpoint?: number) => {
    mocks.playerEntryEnabled = enabled;
    mocks.playerEntryCheckpoint = checkpoint;
    return {
      data: {
        player: mocks.player,
        checkpoint_chapter: checkpoint ?? 2,
        locations: [{ id: 'tower', name: '北塔' }],
      },
      isLoading: false,
      isError: false,
      refetch: vi.fn(),
    };
  },
  useCreatePlayerEntity: () => ({
    mutateAsync: mocks.createPlayer,
    isPending: false,
    isError: false,
  }),
  useWorldState: () => ({ data: undefined }),
  useOpenWorld: () => ({
    data: mocks.openWorld,
    isLoading: false,
    isError: false,
    refetch: vi.fn(),
  }),
  useStartOpenWorld: () => ({
    mutate: mocks.startWorld,
    isPending: false,
    isError: false,
  }),
  useSubmitNarrativeChoice: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
}));

vi.mock('@/widgets/chat-panel/ui/ChatPanel', () => ({
  ChatPanel: ({ character }: { character: { name: string } }) => (
    <div data-testid="chat-panel">{character.name}</div>
  ),
}));
vi.mock('@/widgets/branch-choice/ui/BranchChoice', () => ({
  BranchChoice: () => <div data-testid="branch-choice" />,
}));
vi.mock('@/widgets/world-dashboard/ui/WorldDashboard', () => ({ WorldDashboard: () => null }));

describe('ReaderPage progress gate', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.progressSaving = false;
    mocks.progressError = true;
    mocks.progressChapter = 1;
    mocks.identityType = 'self';
    mocks.hasBranch = false;
    mocks.player = { id: 'player', name: '云舟' };
    mocks.playerEntryEnabled = false;
    mocks.playerEntryCheckpoint = undefined;
    mocks.branchEnabled = false;
    mocks.branchNode = undefined;
    mocks.openWorld = null;
    mocks.characters = [];
    mocks.effectiveContent = 'Chapter two';
    mocks.effectiveGenerated = false;
    mocks.effectiveError = false;
  });

  it('offers an explicit retry after persistence fails', async () => {
    render(<ReaderPage />);
    await waitFor(() => expect(mocks.mutate).toHaveBeenCalledWith(2));

    fireEvent.click(screen.getByRole('button', { name: '重试' }));

    expect(mocks.reset).toHaveBeenCalledOnce();
    expect(mocks.mutate).toHaveBeenCalledTimes(2);
  });

  it('serializes normal chapter navigation while progress is pending', () => {
    mocks.progressSaving = true;
    mocks.progressError = false;
    render(<ReaderPage />);

    expect(screen.getByRole('button', { name: '上一章' }).hasAttribute('disabled')).toBe(true);
    expect(screen.getByRole('button', { name: '下一章' }).hasAttribute('disabled')).toBe(true);
    expect(mocks.mutate).not.toHaveBeenCalled();
  });

  it('closes chat when rewind makes the active character unavailable', async () => {
    mocks.progressChapter = 2;
    mocks.progressError = false;
    mocks.characters = [{
      id: 'future',
      novel_id: 'novel',
      name: 'Future',
      aliases: [],
      role: 'supporting',
      avatar_status: 'pending',
      first_appearance_chapter: 2,
    }];
    const view = render(<ReaderPage />);

    fireEvent.click(screen.getByRole('button', { name: '角色' }));
    fireEvent.click(screen.getByRole('button', { name: /Future/ }));
    expect(screen.getByTestId('chat-panel').textContent).toBe('Future');

    mocks.characters = [];
    view.rerender(<ReaderPage />);
    await waitFor(() => expect(screen.queryByTestId('chat-panel')).toBeNull());
  });

  it('closes and disables chat when the committed timeline kills a character', async () => {
    mocks.progressChapter = 2;
    mocks.progressError = false;
    mocks.characters = [{
      id: 'future',
      novel_id: 'novel',
      name: 'Future',
      aliases: [],
      role: 'supporting',
      avatar_status: 'pending',
      first_appearance_chapter: 2,
    }];
    const view = render(<ReaderPage />);

    fireEvent.click(screen.getByRole('button', { name: '角色' }));
    fireEvent.click(screen.getByRole('button', { name: /Future/ }));
    expect(screen.getByTestId('chat-panel')).toBeTruthy();

    mocks.openWorld = {
      session: { dead_character_ids: ['future'] },
    } as unknown as OpenWorldView;
    view.rerender(<ReaderPage />);
    await waitFor(() => expect(screen.queryByTestId('chat-panel')).toBeNull());

    fireEvent.click(screen.getByRole('button', { name: '角色' }));
    expect(screen.getByRole('button', { name: /Future/ }).hasAttribute('disabled')).toBe(true);
    expect(screen.getByText('当前时间线已死亡')).toBeTruthy();
  });

  it('requires a durable player before enabling self-mode branches', async () => {
    mocks.progressChapter = 2;
    mocks.progressError = false;
    mocks.hasBranch = true;
    mocks.player = null;

    render(<ReaderPage />);

    expect(screen.getByRole('heading', { name: '创建你的原创角色' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '请先创建角色' }).hasAttribute('disabled')).toBe(true);
    expect(mocks.branchEnabled).toBe(false);

    fireEvent.change(screen.getByLabelText('名字'), { target: { value: '云舟' } });
    fireEvent.change(screen.getByLabelText('背景'), { target: { value: '来自边城的地图学徒。' } });
    fireEvent.change(screen.getByLabelText('能力（用逗号分隔）'), { target: { value: '识图，追踪' } });
    fireEvent.change(screen.getByLabelText('随身物品（可选，用逗号分隔）'), { target: { value: '旧地图' } });
    fireEvent.click(screen.getByRole('button', { name: '进入故事' }));

    await waitFor(() => expect(mocks.createPlayer).toHaveBeenCalledWith({
      checkpoint_chapter: 2,
      name: '云舟',
      background: '来自边城的地图学徒。',
      capabilities: ['识图', '追踪'],
      location_id: 'tower',
      inventory: ['旧地图'],
    }));
  });

  it('lets a completed reader choose an earlier unlocked entry checkpoint', async () => {
    mocks.progressChapter = 3;
    mocks.progressError = false;
    mocks.player = null;

    render(<ReaderPage />);
    expect(mocks.playerEntryCheckpoint).toBe(3);
    expect(screen.getByRole('option', { name: '第 1 章' })).toBeTruthy();

    fireEvent.change(screen.getByLabelText('入场章节'), { target: { value: '1' } });
    await waitFor(() => expect(mocks.playerEntryCheckpoint).toBe(1));
  });

  it('keeps character-identity branches compatible without PlayerEntity', () => {
    mocks.progressChapter = 2;
    mocks.progressError = false;
    mocks.identityType = 'character';
    mocks.hasBranch = true;
    mocks.player = null;

    render(<ReaderPage />);

    expect(mocks.playerEntryEnabled).toBe(false);
    expect(mocks.branchEnabled).toBe(true);
    expect(screen.queryByRole('heading', { name: '创建你的原创角色' })).toBeNull();
  });

  it('does not require an uncommitted legacy choice after open-world entry', () => {
    mocks.progressChapter = 2;
    mocks.progressError = false;
    mocks.hasBranch = true;
    mocks.branchNode = {
      id: 'node',
      novel_id: 'novel',
      chapter_number: 2,
      description: '旧分支',
      choices: [{ index: 0, text: '旧选择', hint: '旧路径' }],
    };
    mocks.openWorld = {
      session: { dead_character_ids: [] },
    } as unknown as OpenWorldView;

    render(<ReaderPage />);

    expect(screen.queryByTestId('branch-choice')).toBeNull();
    expect(screen.getByRole('button', { name: '下一章' }).hasAttribute('disabled')).toBe(false);
  });
});

describe('splitChapterAtAnchor', () => {
  it('pauses the canonical chapter immediately after the exact source anchor', () => {
    const result = splitChapterAtAnchor('原文开始。关键事件发生。原著后续。', '关键事件发生。');

    expect(result).toEqual({
      before: '原文开始。关键事件发生。',
      after: '原著后续。',
      anchored: true,
    });
  });

  it('renders the complete player timeline chapter after causality diverges', () => {
    mocks.progressChapter = 2;
    mocks.progressError = false;
    mocks.effectiveContent = '你推开旧城门，原著从未发生的战争由此开始。';
    mocks.effectiveGenerated = true;

    render(<ReaderPage />);

    expect(screen.getByText('你推开旧城门，原著从未发生的战争由此开始。')).toBeTruthy();
    expect(screen.getByText('玩家时间线 · 本章已因你的选择完全改写')).toBeTruthy();
  });

  it('fails open to the full chapter when a legacy anchor is unavailable', () => {
    expect(splitChapterAtAnchor('完整原文', '不存在的锚点')).toEqual({
      before: '完整原文',
      after: '',
      anchored: false,
    });
  });
});
