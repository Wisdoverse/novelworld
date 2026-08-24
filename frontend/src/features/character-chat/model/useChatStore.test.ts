import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { ChatStreamOptions } from '@/shared/api/client';

const streams = vi.hoisted(() => [] as Array<ChatStreamOptions & { cancel: ReturnType<typeof vi.fn> }>);

vi.mock('@/shared/api/client', () => ({
  createChatStream: vi.fn((options: ChatStreamOptions) => {
    const cancel = vi.fn();
    streams.push({ ...options, cancel });
    return cancel;
  }),
}));

import { chatSessionKey, useChatStore } from './useChatStore';

const SELF_SESSION = chatSessionKey('character', 'self');

function send(message = 'private', readerIdentityScope = 'self') {
  useChatStore.getState().sendMessage({
    characterId: 'character',
    novelId: 'novel',
    message,
    readerIdentityScope,
    currentChapter: 1,
  });
}

describe('chat turn isolation', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    streams.length = 0;
    let uuid = 0;
    vi.spyOn(crypto, 'randomUUID').mockImplementation(
      () => `00000000-0000-4000-8000-${String(++uuid).padStart(12, '0')}`,
    );
    useChatStore.setState({
      messages: {},
      streamingText: {},
      isStreaming: {},
      cancelStream: {},
      activeTurnId: {},
      activeTurn: {},
      failedTurn: {},
      sessionGeneration: 0,
    });
  });

  it('retries a failed turn with the same id and without duplicating the optimistic user message', () => {
    send();
    const first = streams[0];
    first.onChunk('partial');
    first.onError({ code: 'stream_error', message: 'try again' });

    useChatStore.getState().retryMessage(SELF_SESSION);
    const retry = streams[1];

    expect(retry.turnId).toBe(first.turnId);
    expect(useChatStore.getState().messages[SELF_SESSION]).toHaveLength(1);
    retry.onRetry?.();
    expect(useChatStore.getState().streamingText[SELF_SESSION]).toBe('');
    retry.onChunk('complete');
    retry.onDone({ turnId: first.turnId, replayed: true, legacy: false });
    expect(useChatStore.getState().messages[SELF_SESSION].map(message => message.role)).toEqual([
      'user',
      'character',
    ]);
  });

  it('ignores late callbacks from a cancelled turn after a new turn starts', () => {
    send('first');
    const first = streams[0];
    send('second');
    const second = streams[1];

    expect(first.cancel).toHaveBeenCalledOnce();
    first.onChunk('late secret');
    first.onDone({ turnId: first.turnId, replayed: false, legacy: false });
    second.onChunk('current');

    expect(useChatStore.getState().streamingText[SELF_SESSION]).toBe('current');
    expect(useChatStore.getState().messages[SELF_SESSION]).toHaveLength(2);
  });

  it('keeps a cancelled turn retryable and ignores its late callbacks', () => {
    send();
    const stream = streams[0];

    useChatStore.getState().cancelMessage(SELF_SESSION);
    stream.onChunk('late secret');
    stream.onDone({ turnId: stream.turnId, replayed: false, legacy: false });

    const state = useChatStore.getState();
    expect(stream.cancel).toHaveBeenCalledOnce();
    expect(state.streamingText[SELF_SESSION]).toBe('');
    expect(state.messages[SELF_SESSION]).toHaveLength(1);
    expect(state.failedTurn[SELF_SESSION]?.turnId).toBe(stream.turnId);
  });

  it('cancels active streams and ignores their late callbacks after reset', () => {
    send();
    const stream = streams[0];

    useChatStore.getState().reset();
    stream.onChunk('late secret');
    stream.onDone({ turnId: stream.turnId, replayed: false, legacy: false });

    expect(stream.cancel).toHaveBeenCalledOnce();
    expect(useChatStore.getState().messages).toEqual({});
    expect(useChatStore.getState().streamingText).toEqual({});
  });

  it('isolates self and character identities while retaining the same character identity session', () => {
    const characterASession = chatSessionKey('character', 'character:a');
    const characterBSession = chatSessionKey('character', 'character:b');

    send('self-only');
    send('character-a-only', 'character:a');

    expect(useChatStore.getState().messages[SELF_SESSION].map(item => item.content))
      .toEqual(['self-only']);
    expect(useChatStore.getState().messages[characterASession].map(item => item.content))
      .toEqual(['character-a-only']);
    expect(useChatStore.getState().messages[characterBSession]).toBeUndefined();

    send('same-character', 'character:a');

    expect(useChatStore.getState().messages[characterASession].map(item => item.content))
      .toEqual(['character-a-only', 'same-character']);
    expect(useChatStore.getState().messages[SELF_SESSION].map(item => item.content))
      .toEqual(['self-only']);
  });
});
