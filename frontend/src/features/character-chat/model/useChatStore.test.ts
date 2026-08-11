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

import { useChatStore } from './useChatStore';

function send(message = 'private') {
  useChatStore.getState().sendMessage({
    characterId: 'character',
    novelId: 'novel',
    message,
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

    useChatStore.getState().retryMessage('character');
    const retry = streams[1];

    expect(retry.turnId).toBe(first.turnId);
    expect(useChatStore.getState().messages.character).toHaveLength(1);
    retry.onRetry?.();
    expect(useChatStore.getState().streamingText.character).toBe('');
    retry.onChunk('complete');
    retry.onDone({ turnId: first.turnId, replayed: true, legacy: false });
    expect(useChatStore.getState().messages.character.map(message => message.role)).toEqual([
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

    expect(useChatStore.getState().streamingText.character).toBe('current');
    expect(useChatStore.getState().messages.character).toHaveLength(2);
  });

  it('keeps a cancelled turn retryable and ignores its late callbacks', () => {
    send();
    const stream = streams[0];

    useChatStore.getState().cancelMessage('character');
    stream.onChunk('late secret');
    stream.onDone({ turnId: stream.turnId, replayed: false, legacy: false });

    const state = useChatStore.getState();
    expect(stream.cancel).toHaveBeenCalledOnce();
    expect(state.streamingText.character).toBe('');
    expect(state.messages.character).toHaveLength(1);
    expect(state.failedTurn.character?.turnId).toBe(stream.turnId);
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
});
