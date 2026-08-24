import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { NarrativeNode, OpenWorldView } from '@/shared/types';
import { ReaderPage, splitChapterAtAnchor } from './ReaderPage';

const mocks = vi.hoisted(() => ({
  novelId: 'novel',
  navigate: vi.fn(),
  mutate: vi.fn(),
  reset: vi.fn(),
  routeChapter: '2',
  progressSaving: false,
  progressError: true,
  progressChapter: 1,
  identityType: 'self',
  readerIdentity: undefined as string | undefined,
  readerCharacterId: undefined as string | undefined,
  hasBranch: false,
  player: {
    id: 'player', name: '云舟', canonical_checkpoint_chapter: 1, location_id: 'tower',
  } as Record<string, unknown> | null,
  playerEntryEnabled: false,
  playerEntryCheckpoint: undefined as number | undefined,
  playerEntryError: false,
  branchEnabled: false,
  branchNode: undefined as NarrativeNode | undefined,
  createPlayer: vi.fn(),
  submitChoice: vi.fn(),
  startWorld: vi.fn(),
  openWorld: null as OpenWorldView | null,
  openWorldError: false,
  refetchOpenWorld: vi.fn(),
  characters: [] as Array<Record<string, unknown>>,
  charactersChapter: 0,
  effectiveContent: 'Chapter two',
  effectiveGenerated: false,
  effectiveError: false,
  translationContent: '第二章',
  translationError: false,
  effectiveIdentityScope: 'unresolved',
  effectiveProgressBoundary: 0,
  effectiveEnabled: false,
  effectiveByQuery: {} as Record<string, {
    content: string;
    generated: boolean;
    isLoading?: boolean;
    isError?: boolean;
  }>,
  refetchEffectiveChapter: vi.fn(),
  worldChoices: [] as Array<{
    node_id: string;
    chapter?: number;
    choice_index: number;
    choice?: string;
    consequence?: string;
  }>,
  worldPlayerCheckpoint: undefined as number | undefined,
  worldOpenWorldCheckpoint: undefined as number | undefined,
  refetchWorldState: vi.fn(),
}));

vi.mock('react-router-dom', () => ({
  useNavigate: () => mocks.navigate,
  useParams: () => ({ novelId: mocks.novelId, chapterNum: mocks.routeChapter }),
}));

vi.mock('@/entities/novel/api', () => ({
  useNovel: () => ({ data: { id: mocks.novelId, title: 'Novel', total_chapters: 3 } }),
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
  useCharacters: (_novelId: string, chapter: number) => {
    mocks.charactersChapter = chapter;
    return {
      data: mocks.characters.filter(character => (
        typeof character.first_appearance_chapter !== 'number'
          || character.first_appearance_chapter <= chapter
      )),
    };
  },
}));

vi.mock('@/entities/reading-progress/api', () => ({
  useReadingProgress: () => ({
    data: {
      current_chapter: mocks.progressChapter,
      reader_identity: mocks.readerIdentity,
      reader_identity_type: mocks.identityType,
      reader_character_id: mocks.readerCharacterId,
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

vi.mock('@/features/chapter-translation', () => ({
  useChapterTranslation: (
    _novelId: string,
    _chapterNumber: number,
    _content: string,
    enabled: boolean,
  ) => ({
    data: enabled && !mocks.translationError ? { content: mocks.translationContent } : undefined,
    isFetching: false,
    isError: enabled && mocks.translationError,
    refetch: vi.fn(),
  }),
  TranslationControls: ({
    active,
    isError,
    onToggle,
  }: {
    active: boolean;
    isError: boolean;
    onToggle: () => void;
  }) => (
    <>
      <button onClick={onToggle}>{active ? '显示原文' : '翻译成中文'}</button>
      {isError ? <span role="alert">翻译失败，当前显示原文。</span> : null}
    </>
  ),
}));

vi.mock('@/entities/narrative/api', () => ({
  useEffectiveChapter: (
    _novelId: string,
    _chapter: number,
    identityScope: string,
    progressBoundary: number,
    enabled: boolean,
  ) => {
    mocks.effectiveIdentityScope = identityScope;
    mocks.effectiveProgressBoundary = progressBoundary;
    mocks.effectiveEnabled = enabled;
    const scoped = mocks.effectiveByQuery[`${identityScope}:${progressBoundary}`];
    const isError = scoped?.isError ?? mocks.effectiveError;
    return {
      data: isError ? undefined : {
      chapter_number: 2,
        content: scoped?.content ?? mocks.effectiveContent,
        generated: scoped?.generated ?? mocks.effectiveGenerated,
      },
      isLoading: scoped?.isLoading ?? false,
      isError,
      refetch: mocks.refetchEffectiveChapter,
    };
  },
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
      data: mocks.playerEntryError ? undefined : {
        player: mocks.player,
        checkpoint_chapter: checkpoint ?? 2,
        locations: [{ id: 'tower', name: '北塔' }],
      },
      isLoading: false,
      isError: mocks.playerEntryError,
      refetch: vi.fn(),
    };
  },
  useCreatePlayerEntity: () => ({
    mutateAsync: mocks.createPlayer,
    isPending: false,
    isError: false,
  }),
  useGenerateGameRules: () => ({
    mutate: vi.fn(),
    isPending: false,
    isError: false,
    error: null,
  }),
  useWorldState: () => ({
    data: {
      state: {
        choices: mocks.worldChoices,
        player_entity: mocks.worldPlayerCheckpoint === undefined ? undefined : {
          canonical_checkpoint_chapter: mocks.worldPlayerCheckpoint,
        },
        open_world: mocks.worldOpenWorldCheckpoint === undefined ? undefined : {
          entry_context: { unlocked_through_chapter: mocks.worldOpenWorldCheckpoint },
        },
      },
    },
    refetch: mocks.refetchWorldState,
  }),
  useOpenWorld: () => ({
    data: mocks.openWorld,
    isLoading: false,
    isError: mocks.openWorldError,
    refetch: mocks.refetchOpenWorld,
  }),
  useStartOpenWorld: () => ({
    mutate: mocks.startWorld,
    isPending: false,
    isError: false,
  }),
  useSubmitNarrativeChoice: () => ({
    mutateAsync: mocks.submitChoice,
    isPending: false,
  }),
  isNarrativeChoiceConflict: (error: { response?: { data?: { error?: { code?: string } } } }) => (
    error.response?.data?.error?.code === 'choice_conflict'
  ),
}));

vi.mock('@/widgets/chat-panel/ui/ChatPanel', () => ({
  ChatPanel: ({ character }: { character: { name: string } }) => (
    <div data-testid="chat-panel">{character.name}</div>
  ),
}));
vi.mock('@/widgets/branch-choice/ui/BranchChoice', () => ({
  BranchChoice: ({
    node,
    onChoose,
    selectedChoiceIndex,
    error,
    isRecoveryLocked,
    onRetryRecovery,
  }: {
    node: NarrativeNode;
    onChoose: (choice: NarrativeNode['choices'][number]) => Promise<void>;
    selectedChoiceIndex?: number;
    error?: string;
    isRecoveryLocked?: boolean;
    onRetryRecovery?: () => Promise<void>;
  }) => (
    <div data-testid="branch-choice">
      <span data-testid="selected-choice">{selectedChoiceIndex ?? 'none'}</span>
      {node.choices[1] ? (
        <button
          type="button"
          disabled={isRecoveryLocked}
          onClick={() => void onChoose(node.choices[1]).catch(() => undefined)}
        >
          选择第二项
        </button>
      ) : null}
      {error ? (
        <div role="alert">
          {error}
          {isRecoveryLocked && onRetryRecovery ? (
            <button type="button" onClick={() => void onRetryRecovery()}>重新加载已提交结果</button>
          ) : null}
        </div>
      ) : null}
    </div>
  ),
}));
vi.mock('@/widgets/world-dashboard/ui/WorldDashboard', () => ({
  WorldDashboard: ({ actionsDisabled }: { actionsDisabled?: boolean }) => (
    <button id="world-action-journal" disabled={actionsDisabled}>模拟世界行动</button>
  ),
}));

describe('ReaderPage progress gate', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.novelId = 'novel';
    mocks.routeChapter = '2';
    mocks.progressSaving = false;
    mocks.progressError = true;
    mocks.progressChapter = 1;
    mocks.identityType = 'self';
    mocks.readerIdentity = undefined;
    mocks.readerCharacterId = undefined;
    mocks.hasBranch = false;
    mocks.player = {
      id: 'player', name: '云舟', canonical_checkpoint_chapter: 1, location_id: 'tower',
    };
    mocks.playerEntryEnabled = false;
    mocks.playerEntryCheckpoint = undefined;
    mocks.playerEntryError = false;
    mocks.branchEnabled = false;
    mocks.branchNode = undefined;
    mocks.openWorld = null;
    mocks.openWorldError = false;
    mocks.characters = [];
    mocks.charactersChapter = 0;
    mocks.effectiveContent = 'Chapter two';
    mocks.effectiveGenerated = false;
    mocks.effectiveError = false;
    mocks.translationContent = '第二章';
    mocks.translationError = false;
    mocks.effectiveIdentityScope = 'unresolved';
    mocks.effectiveProgressBoundary = 0;
    mocks.effectiveEnabled = false;
    mocks.effectiveByQuery = {};
    mocks.worldChoices = [];
    mocks.worldPlayerCheckpoint = undefined;
    mocks.worldOpenWorldCheckpoint = undefined;
    mocks.submitChoice.mockReset();
    mocks.refetchWorldState.mockReset();
    mocks.refetchOpenWorld.mockReset();
    mocks.refetchWorldState.mockImplementation(async () => ({
      data: { state: { choices: mocks.worldChoices } },
    }));
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

  it('uses the shared light app surface for the reader', () => {
    mocks.progressError = false;

    const { container } = render(<ReaderPage />);

    expect(container.firstElementChild?.classList.contains('app-surface')).toBe(true);
  });

  it('switches the visible chapter body between the source and Chinese translation', () => {
    mocks.progressError = false;
    render(<ReaderPage />);

    expect(screen.getByText('Chapter two')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '翻译成中文' }));
    expect(screen.getByText('第二章')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '显示原文' }));
    expect(screen.getByText('Chapter two')).toBeTruthy();
  });

  it('keeps the source visible when translation fails', () => {
    mocks.progressError = false;
    mocks.translationError = true;
    render(<ReaderPage />);

    fireEvent.click(screen.getByRole('button', { name: '翻译成中文' }));

    expect(screen.getByText('Chapter two')).toBeTruthy();
    expect(screen.getByRole('alert').textContent).toContain('当前显示原文');
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

  it('hides a future character synchronously while a rewind is being committed', () => {
    mocks.routeChapter = '5';
    mocks.progressChapter = 5;
    mocks.progressSaving = false;
    mocks.progressError = false;
    mocks.characters = [{
      id: 'future-five',
      novel_id: 'novel',
      name: 'Future Five',
      aliases: [],
      role: 'supporting',
      avatar_status: 'pending',
      first_appearance_chapter: 5,
    }];
    const page = render(<ReaderPage />);

    fireEvent.click(screen.getByRole('button', { name: '角色' }));
    fireEvent.click(screen.getByRole('button', { name: /Future Five/ }));
    expect(screen.getByTestId('chat-panel').textContent).toBe('Future Five');

    mocks.routeChapter = '2';
    mocks.progressSaving = true;
    page.rerender(<ReaderPage />);

    expect(mocks.charactersChapter).toBe(2);
    expect(screen.queryByTestId('chat-panel')).toBeNull();
    expect(screen.queryByText('Future Five')).toBeNull();
    expect(screen.getByRole('button', { name: '角色' }).hasAttribute('disabled')).toBe(true);
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

  it('allows canon reading but requires a durable player before self-mode branches', async () => {
    mocks.progressChapter = 2;
    mocks.progressError = false;
    mocks.hasBranch = true;
    mocks.player = null;

    render(<ReaderPage />);

    expect(screen.getByRole('heading', { name: '创建你的原创角色' })).toBeTruthy();
    expect(screen.getByText('行动判定（高级项）')).toBeTruthy();
    expect(screen.getByRole('button', { name: '下一章' }).hasAttribute('disabled')).toBe(false);
    expect(mocks.branchEnabled).toBe(false);
    expect(screen.getByText(/创建后入场点及此前历史不可更改/)).toBeTruthy();

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
      rules: {
        mode: 'narrative',
        canon_model_version: null,
        template_schema_version: null,
        template_prompt_version: null,
        attributes: {},
      },
    }));
  });

  it('resets player-entry state when navigating to another novel', () => {
    mocks.progressError = false;
    mocks.player = null;
    const page = render(<ReaderPage />);

    fireEvent.change(screen.getByLabelText('名字'), { target: { value: '云舟' } });
    expect((screen.getByLabelText('名字') as HTMLInputElement).value).toBe('云舟');

    mocks.novelId = 'another-novel';
    page.rerender(<ReaderPage />);

    expect((screen.getByLabelText('名字') as HTMLInputElement).value).toBe('');
  });

  it('lets a completed reader choose an earlier unlocked entry checkpoint', async () => {
    mocks.routeChapter = '3';
    mocks.progressChapter = 3;
    mocks.progressError = false;
    mocks.player = null;

    render(<ReaderPage />);
    expect(mocks.playerEntryCheckpoint).toBe(3);
    expect(screen.getByRole('option', { name: '第 1 章' })).toBeTruthy();

    fireEvent.change(screen.getByLabelText('入场章节'), { target: { value: '1' } });
    await waitFor(() => expect(mocks.playerEntryCheckpoint).toBe(1));
  });

  it('does not request a new branch for a character identity', () => {
    mocks.progressChapter = 2;
    mocks.progressError = false;
    mocks.identityType = 'character';
    mocks.readerCharacterId = 'character-id';
    mocks.hasBranch = true;
    mocks.player = null;

    render(<ReaderPage />);

    expect(mocks.playerEntryEnabled).toBe(false);
    expect(mocks.branchEnabled).toBe(false);
    expect(screen.queryByRole('heading', { name: '创建你的原创角色' })).toBeNull();
    expect(screen.queryByTestId('branch-choice')).toBeNull();
  });

  it('shows a character identity only its already committed branch replay', () => {
    mocks.progressChapter = 2;
    mocks.progressError = false;
    mocks.identityType = 'character';
    mocks.readerCharacterId = 'character-id';
    mocks.hasBranch = true;
    mocks.player = null;
    mocks.worldChoices = [{
      node_id: 'committed-node',
      chapter: 2,
      choice_index: 0,
      consequence: '已经发生',
    }];
    mocks.branchNode = {
      id: 'committed-node',
      novel_id: 'novel',
      chapter_number: 2,
      description: '已提交分支',
      choices: [{ index: 0, text: '既定选择', hint: '' }],
    };

    render(<ReaderPage />);

    expect(mocks.branchEnabled).toBe(true);
    expect(screen.getByTestId('selected-choice').textContent).toBe('0');
  });

  it('hides cached self-only world data immediately after switching to a character identity', () => {
    mocks.progressChapter = 2;
    mocks.progressError = false;
    mocks.hasBranch = true;
    mocks.effectiveGenerated = true;
    mocks.effectiveContent = '身份切换后的时间线正文';
    mocks.effectiveByQuery['character:character-id:2'] = {
      content: 'Chapter two',
      generated: false,
    };
    mocks.player = {
      id: 'player', name: '云舟', canonical_checkpoint_chapter: 2, location_id: 'tower',
    };
    mocks.openWorld = {
      session: {
        dead_character_ids: [],
        entry_context: { unlocked_through_chapter: 2 },
      },
    } as unknown as OpenWorldView;
    mocks.branchNode = {
      id: 'character-node',
      novel_id: 'novel',
      chapter_number: 2,
      description: '角色视角的选择',
      choices: [{ index: 0, text: '继续调查', hint: '' }],
    };
    const page = render(<ReaderPage />);

    expect(screen.getByRole('button', { name: '模拟世界行动' })).toBeTruthy();
    expect(screen.getByRole('heading', { name: '云舟的故事' })).toBeTruthy();
    expect(screen.queryByTestId('branch-choice')).toBeNull();

    mocks.identityType = 'character';
    mocks.readerCharacterId = 'character-id';
    page.rerender(<ReaderPage />);

    expect(screen.queryByText('身份切换后的时间线正文')).toBeNull();
    expect(screen.getByText('Chapter two')).toBeTruthy();
    expect(screen.queryByRole('button', { name: '模拟世界行动' })).toBeNull();
    expect(screen.queryByRole('heading', { name: '以 云舟 之名，踏入这个世界' })).toBeNull();
    expect(screen.queryByRole('heading', { name: '云舟的故事' })).toBeNull();
    expect(screen.getByRole('heading', { name: 'Two' })).toBeTruthy();
    expect(mocks.effectiveIdentityScope).toBe('character:character-id');
    expect(mocks.effectiveProgressBoundary).toBe(2);
    expect(mocks.playerEntryEnabled).toBe(false);
    expect(mocks.branchEnabled).toBe(false);
    expect(screen.queryByTestId('branch-choice')).toBeNull();
  });

  it('presents open-world entry as the player\'s story rather than a generic notice', () => {
    mocks.progressChapter = 2;
    mocks.progressError = false;
    mocks.player = {
      id: 'player',
      name: '云舟',
      canonical_checkpoint_chapter: 2,
      location_id: 'tower',
    };

    render(<ReaderPage />);

    expect(screen.getByRole('heading', { name: '以 云舟 之名，踏入这个世界' })).toBeTruthy();
    expect(screen.getByText('入场 · 第 2 章')).toBeTruthy();
    expect(screen.getByText('北塔')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '进入开放世界' }));
    expect(mocks.startWorld).toHaveBeenCalledTimes(1);
  });

  it('locks open-world entry until route and persisted progress agree', () => {
    mocks.routeChapter = '1';
    mocks.progressChapter = 5;
    mocks.progressSaving = true;
    mocks.progressError = false;
    mocks.player = {
      id: 'player',
      name: '云舟',
      canonical_checkpoint_chapter: 1,
      location_id: 'tower',
    };
    const page = render(<ReaderPage />);

    const pendingEntry = screen.getByRole('button', { name: '进入开放世界' });
    expect(pendingEntry.hasAttribute('disabled')).toBe(true);
    fireEvent.click(pendingEntry);
    expect(mocks.startWorld).not.toHaveBeenCalled();

    mocks.progressSaving = false;
    mocks.progressChapter = 1;
    page.rerender(<ReaderPage />);

    const settledEntry = screen.getByRole('button', { name: '进入开放世界' });
    expect(settledEntry.hasAttribute('disabled')).toBe(false);
    fireEvent.click(settledEntry);
    expect(mocks.startWorld).toHaveBeenCalledOnce();
  });

  it('locks player creation until route and persisted progress agree', async () => {
    mocks.routeChapter = '5';
    mocks.progressChapter = 5;
    mocks.progressSaving = false;
    mocks.progressError = false;
    mocks.player = null;
    const page = render(<ReaderPage />);
    expect(mocks.playerEntryCheckpoint).toBe(5);

    mocks.routeChapter = '1';
    mocks.progressSaving = true;
    page.rerender(<ReaderPage />);

    const pendingEntry = screen.getByRole('button', { name: '进入故事' });
    expect(pendingEntry.hasAttribute('disabled')).toBe(true);
    expect(mocks.playerEntryCheckpoint).toBe(1);
    for (const label of [
      '入场章节', '名字', '背景', '能力（用逗号分隔）', '初始地点', '随身物品（可选，用逗号分隔）',
    ]) {
      expect((screen.getByLabelText(label) as HTMLInputElement).hasAttribute('disabled')).toBe(true);
    }
    fireEvent.click(pendingEntry);
    expect(mocks.createPlayer).not.toHaveBeenCalled();

    mocks.progressSaving = false;
    mocks.progressChapter = 1;
    page.rerender(<ReaderPage />);

    expect(mocks.playerEntryCheckpoint).toBe(1);
    expect((screen.getByLabelText('入场章节') as HTMLSelectElement).hasAttribute('disabled')).toBe(false);

    fireEvent.change(screen.getByLabelText('名字'), { target: { value: '云舟' } });
    fireEvent.change(screen.getByLabelText('背景'), { target: { value: '来自边城的地图学徒。' } });
    fireEvent.change(screen.getByLabelText('能力（用逗号分隔）'), { target: { value: '识图' } });
    fireEvent.click(screen.getByRole('button', { name: '进入故事' }));

    await waitFor(() => expect(mocks.createPlayer).toHaveBeenCalledWith(expect.objectContaining({
      checkpoint_chapter: 1,
      name: '云舟',
    })));
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
    expect(mocks.branchEnabled).toBe(false);
    expect(screen.getByRole('button', { name: '继续旅程' }).hasAttribute('disabled')).toBe(false);
  });

  it('does not load or require a future branch after the Player checkpoint', () => {
    mocks.progressChapter = 2;
    mocks.progressError = false;
    mocks.hasBranch = true;
    mocks.player = {
      id: 'player',
      name: '云舟',
      canonical_checkpoint_chapter: 1,
      location_id: 'tower',
    };
    mocks.branchNode = {
      id: 'future-node',
      novel_id: 'novel',
      chapter_number: 2,
      description: '不会进入当前时间线的旧分支',
      choices: [{ index: 0, text: '未来选项', hint: '' }],
    };

    render(<ReaderPage />);

    expect(mocks.branchEnabled).toBe(false);
    expect(screen.queryByTestId('branch-choice')).toBeNull();
    expect(screen.getByRole('button', { name: '下一章' }).hasAttribute('disabled')).toBe(false);
  });

  it('disables stale world actions until the open-world retry recovers', async () => {
    mocks.progressChapter = 2;
    mocks.progressError = false;
    mocks.openWorld = {
      session: { dead_character_ids: [] },
    } as unknown as OpenWorldView;
    mocks.openWorldError = true;
    mocks.refetchOpenWorld.mockImplementation(async () => {
      mocks.openWorldError = false;
      return { data: mocks.openWorld };
    });
    const page = render(<ReaderPage />);

    expect(screen.getByRole('button', { name: '模拟世界行动' }).hasAttribute('disabled')).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: '重试' }));
    await waitFor(() => expect(mocks.refetchOpenWorld).toHaveBeenCalledOnce());
    page.rerender(<ReaderPage />);

    expect(screen.getByRole('button', { name: '模拟世界行动' }).hasAttribute('disabled')).toBe(false);
  });

  it('hides cached world content after rewind and restores it at the source high-water', async () => {
    mocks.progressChapter = 1;
    mocks.progressError = false;
    mocks.effectiveGenerated = true;
    mocks.effectiveContent = '回退后不可见的玩家时间线正文';
    mocks.openWorld = {
      session: {
        dead_character_ids: [],
        entry_context: { unlocked_through_chapter: 2 },
      },
    } as unknown as OpenWorldView;
    const page = render(<ReaderPage />);

    expect(screen.queryByRole('button', { name: '模拟世界行动' })).toBeNull();
    expect(screen.queryByText('回退后不可见的玩家时间线正文')).toBeNull();
    expect(screen.getByText('Chapter two')).toBeTruthy();
    expect(screen.getByText(/阅读到第 2 章后/)).toBeTruthy();

    mocks.progressChapter = 2;
    page.rerender(<ReaderPage />);

    await waitFor(() => expect(screen.getByRole('button', { name: '模拟世界行动' })).toBeTruthy());
    expect(mocks.refetchOpenWorld).toHaveBeenCalled();
    expect(mocks.refetchWorldState).toHaveBeenCalled();
  });

  it('hides a cached generated chapter while route and progress disagree', () => {
    mocks.progressChapter = 1;
    mocks.progressError = false;
    mocks.effectiveGenerated = true;
    mocks.effectiveContent = '没有开放世界状态也不能越过回退边界的生成正文';

    render(<ReaderPage />);

    expect(screen.queryByText('没有开放世界状态也不能越过回退边界的生成正文')).toBeNull();
    expect(screen.getByText('Chapter two')).toBeTruthy();
  });

  it('hides derived history synchronously while a route rewind is still being committed', () => {
    mocks.routeChapter = '1';
    mocks.progressChapter = 2;
    mocks.progressSaving = true;
    mocks.progressError = false;
    mocks.effectiveGenerated = true;
    mocks.effectiveContent = '路由回退后必须立即隐藏的玩家时间线正文';
    mocks.openWorld = {
      session: {
        dead_character_ids: [],
        entry_context: { unlocked_through_chapter: 2 },
      },
    } as unknown as OpenWorldView;
    const page = render(<ReaderPage />);

    expect(screen.queryByText('路由回退后必须立即隐藏的玩家时间线正文')).toBeNull();
    expect(screen.queryByRole('button', { name: '模拟世界行动' })).toBeNull();
    expect(screen.getByText('Chapter two')).toBeTruthy();

    mocks.progressSaving = false;
    mocks.progressChapter = 1;
    page.rerender(<ReaderPage />);

    expect(screen.queryByText('路由回退后必须立即隐藏的玩家时间线正文')).toBeNull();
    expect(screen.queryByRole('button', { name: '模拟世界行动' })).toBeNull();
  });

  it('does not reuse a generated chapter from a newer progress boundary after rewind settles', () => {
    mocks.routeChapter = '2';
    mocks.progressChapter = 5;
    mocks.progressSaving = true;
    mocks.progressError = false;
    mocks.effectiveByQuery['self:5'] = {
      content: 'progress-five-only marker',
      generated: true,
    };
    mocks.effectiveByQuery['self:2'] = {
      content: 'Chapter two',
      generated: false,
    };
    const page = render(<ReaderPage />);

    expect(mocks.effectiveEnabled).toBe(false);
    expect(mocks.effectiveProgressBoundary).toBe(5);
    expect(screen.queryByText('progress-five-only marker')).toBeNull();
    expect(screen.getByText('Chapter two')).toBeTruthy();

    mocks.progressSaving = false;
    mocks.progressChapter = 2;
    page.rerender(<ReaderPage />);

    expect(mocks.effectiveEnabled).toBe(true);
    expect(mocks.effectiveProgressBoundary).toBe(2);
    expect(screen.queryByText('progress-five-only marker')).toBeNull();
    expect(screen.getByText('Chapter two')).toBeTruthy();
  });

  it('uses cached world state when the rewound Player query is rejected', async () => {
    mocks.progressChapter = 2;
    mocks.progressError = false;
    mocks.effectiveGenerated = true;
    mocks.effectiveContent = 'Player 查询回退期间不可见的正文';
    mocks.worldPlayerCheckpoint = 2;
    const page = render(<ReaderPage />);

    expect(screen.getByText('Player 查询回退期间不可见的正文')).toBeTruthy();
    mocks.progressChapter = 1;
    mocks.playerEntryError = true;
    page.rerender(<ReaderPage />);

    expect(screen.queryByText('Player 查询回退期间不可见的正文')).toBeNull();
    expect(screen.getByText('Chapter two')).toBeTruthy();
    await waitFor(() => expect(mocks.refetchWorldState).toHaveBeenCalled());
  });

  it('uses cached choices to hide generated prose for a rewound character identity', async () => {
    mocks.progressChapter = 2;
    mocks.progressError = false;
    mocks.identityType = 'character';
    mocks.player = null;
    mocks.effectiveGenerated = true;
    mocks.effectiveContent = '角色身份回退后不可见的正文';
    mocks.worldChoices = [{
      node_id: 'choice-at-two',
      chapter: 2,
      choice_index: 0,
      choice: '岔路',
      consequence: '改变未来',
    }];
    const page = render(<ReaderPage />);

    expect(screen.getByText('角色身份回退后不可见的正文')).toBeTruthy();
    mocks.progressChapter = 1;
    page.rerender(<ReaderPage />);

    expect(screen.queryByText('角色身份回退后不可见的正文')).toBeNull();
    expect(screen.getByText('Chapter two')).toBeTruthy();
    await waitFor(() => expect(mocks.refetchWorldState).toHaveBeenCalled());
  });

  it('hides pre-open world state when progress is behind the Player checkpoint', () => {
    mocks.progressChapter = 1;
    mocks.progressError = false;
    mocks.hasBranch = true;
    mocks.player = {
      id: 'player',
      name: '云舟',
      canonical_checkpoint_chapter: 2,
      location_id: 'tower',
    };
    mocks.branchNode = {
      id: 'rewound-node',
      novel_id: 'novel',
      chapter_number: 2,
      description: '回退后不可见的分支',
      choices: [{ index: 0, text: '改写未来', hint: '' }],
    };
    mocks.worldChoices = [{
      node_id: 'rewound-node',
      chapter: 2,
      choice_index: 0,
      choice: '改写未来',
      consequence: '不应显示',
    }];

    render(<ReaderPage />);

    expect(mocks.branchEnabled).toBe(false);
    expect(screen.queryByTestId('branch-choice')).toBeNull();
    expect(screen.queryByRole('heading', { name: '以 云舟 之名，踏入这个世界' })).toBeNull();
    expect(screen.getByText(/阅读到第 2 章后/)).toBeTruthy();
  });

  it('settles a choice conflict on the committed server state and clears recovery copy', async () => {
    mocks.progressChapter = 2;
    mocks.progressError = false;
    mocks.hasBranch = true;
    mocks.player = {
      id: 'player', name: '云舟', canonical_checkpoint_chapter: 2, location_id: 'tower',
    };
    mocks.branchNode = {
      id: 'node',
      novel_id: 'novel',
      chapter_number: 2,
      description: '岔路',
      choices: [
        { index: 0, text: '另一窗口已提交', hint: '' },
        { index: 1, text: '本窗口请求', hint: '' },
      ],
    };
    mocks.submitChoice.mockRejectedValue({
      response: { data: { error: { code: 'choice_conflict' } } },
    });

    const page = render(<ReaderPage />);
    fireEvent.click(screen.getByRole('button', { name: '选择第二项' }));
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('另一窗口已经提交'));
    expect(screen.getByRole('button', { name: '选择第二项' }).hasAttribute('disabled')).toBe(true);

    mocks.worldChoices = [{ node_id: 'node', choice_index: 0, consequence: '权威结果' }];
    fireEvent.click(screen.getByRole('button', { name: '重新加载已提交结果' }));
    await waitFor(() => expect(mocks.refetchWorldState).toHaveBeenCalledOnce());
    page.rerender(<ReaderPage />);

    await waitFor(() => expect(screen.getByTestId('selected-choice').textContent).toBe('0'));
    await waitFor(() => expect(screen.queryByRole('alert')).toBeNull());
  });

  it('reviews the world journal without changing the source chapter', () => {
    mocks.progressChapter = 2;
    mocks.progressError = false;
    mocks.openWorld = {
      session: { dead_character_ids: [] },
    } as unknown as OpenWorldView;
    const originalScrollIntoView = Element.prototype.scrollIntoView;
    const scrollIntoView = vi.fn();
    Object.defineProperty(Element.prototype, 'scrollIntoView', {
      configurable: true,
      value: scrollIntoView,
    });
    try {
      render(<ReaderPage />);
      fireEvent.click(screen.getByRole('button', { name: '回看行动日志' }));

      expect(scrollIntoView).toHaveBeenCalledOnce();
      expect(mocks.navigate).not.toHaveBeenCalled();
    } finally {
      if (originalScrollIntoView) {
        Object.defineProperty(Element.prototype, 'scrollIntoView', {
          configurable: true,
          value: originalScrollIntoView,
        });
      } else {
        Reflect.deleteProperty(Element.prototype, 'scrollIntoView');
      }
    }
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
    expect(screen.getByText('原著坐标 · 第 2 章《Two》')).toBeTruthy();
    expect(screen.getByRole('heading', { name: '云舟的故事' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '回看行动日志' })).toBeTruthy();
    expect(screen.getByRole('button', { name: '继续旅程' })).toBeTruthy();
  });

  it('keeps the immutable source available without mixing it into player history', () => {
    mocks.progressChapter = 2;
    mocks.progressError = false;
    mocks.effectiveContent = '你推开旧城门，原著从未发生的战争由此开始。';
    mocks.effectiveGenerated = true;

    render(<ReaderPage />);
    fireEvent.click(screen.getByRole('button', { name: '原著参考' }));

    expect(screen.getByText('Chapter two')).toBeTruthy();
    expect(screen.queryByText('你推开旧城门，原著从未发生的战争由此开始。')).toBeNull();
    expect(screen.getByText('这是原著内容，仅用于回看世界设定，不属于你当前时间线已经发生的历史。')).toBeTruthy();
  });

  it('fails open to the full chapter when a legacy anchor is unavailable', () => {
    expect(splitChapterAtAnchor('完整原文', '不存在的锚点')).toEqual({
      before: '完整原文',
      after: '',
      anchored: false,
    });
  });
});
