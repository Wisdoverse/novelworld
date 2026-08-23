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
  submitChoice: vi.fn(),
  startWorld: vi.fn(),
  openWorld: null as OpenWorldView | null,
  characters: [] as Array<Record<string, unknown>>,
  effectiveContent: 'Chapter two',
  effectiveGenerated: false,
  effectiveError: false,
  translationContent: '第二章',
  translationError: false,
  worldChoices: [] as Array<{ node_id: string; choice_index: number; consequence?: string }>,
  refetchWorldState: vi.fn(),
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
      {active && isError ? <span role="alert">翻译失败，当前显示原文。</span> : null}
    </>
  ),
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
  useWorldState: () => ({
    data: { state: { choices: mocks.worldChoices } },
    refetch: mocks.refetchWorldState,
  }),
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
  WorldDashboard: () => <div id="world-action-journal" />,
}));

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
    mocks.translationContent = '第二章';
    mocks.translationError = false;
    mocks.worldChoices = [];
    mocks.submitChoice.mockReset();
    mocks.refetchWorldState.mockReset();
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

  it('presents open-world entry as the player\'s story rather than a generic notice', () => {
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
    expect(screen.getByRole('button', { name: '继续旅程' }).hasAttribute('disabled')).toBe(false);
  });

  it('settles a choice conflict on the committed server state and clears recovery copy', async () => {
    mocks.progressChapter = 2;
    mocks.progressError = false;
    mocks.hasBranch = true;
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
