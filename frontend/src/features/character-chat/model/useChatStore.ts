import { create } from 'zustand';

import {
  createChatStream,
  type ChatStreamError,
  type ChatStreamPayload,
} from '@/shared/api/client';
import type { ChatMessage } from '@/shared/types';

interface ChatTurn {
  turnId: string;
  sessionKey: string;
  characterId: string;
  payload: ChatStreamPayload;
}

interface FailedChatTurn extends ChatTurn {
  error: ChatStreamError;
}

interface ChatState {
  messages: Record<string, ChatMessage[]>;
  streamingText: Record<string, string>;
  isStreaming: Record<string, boolean>;
  cancelStream: Record<string, (() => void) | null>;
  activeTurnId: Record<string, string | undefined>;
  activeTurn: Record<string, ChatTurn | undefined>;
  failedTurn: Record<string, FailedChatTurn | undefined>;
  sessionGeneration: number;

  sendMessage: (params: {
    characterId: string;
    novelId: string;
    message: string;
    readerIdentity?: string;
    readerIdentityScope: string;
    currentChapter: number;
  }) => boolean;
  retryMessage: (sessionKey: string) => void;
  cancelMessage: (sessionKey: string, expectedTurnId?: string) => void;
  addMessage: (sessionKey: string, message: ChatMessage) => void;
  clearMessages: (sessionKey: string) => void;
  reset: () => void;
}

export const chatSessionKey = (characterId: string, readerIdentityScope: string) =>
  `${characterId}:${readerIdentityScope}`;

export const useChatStore = create<ChatState>((set, get) => {
  const startTurn = (turn: ChatTurn, generation: number) => {
    const { characterId, sessionKey, turnId, payload } = turn;
    const cancel = createChatStream({
      characterId,
      turnId,
      payload,
      onChunk: chunk => set(state => {
        if (
          state.sessionGeneration !== generation
          || state.activeTurnId[sessionKey] !== turnId
        ) return state;
        return {
          streamingText: {
            ...state.streamingText,
            [sessionKey]: (state.streamingText[sessionKey] || '') + chunk,
          },
        };
      }),
      onRetry: () => set(state => {
        if (
          state.sessionGeneration !== generation
          || state.activeTurnId[sessionKey] !== turnId
        ) return state;
        return {
          streamingText: { ...state.streamingText, [sessionKey]: '' },
        };
      }),
      onDone: result => set(state => {
        if (
          result.turnId !== turnId
          || state.sessionGeneration !== generation
          || state.activeTurnId[sessionKey] !== turnId
        ) return state;
        const finalText = state.streamingText[sessionKey] || '';
        const characterMessage: ChatMessage = {
          id: crypto.randomUUID(),
          turn_id: turnId,
          role: 'character',
          content: finalText,
          character_id: characterId,
          chapter_context: payload.current_chapter,
          created_at: new Date().toISOString(),
        };
        return {
          messages: {
            ...state.messages,
            [sessionKey]: [...(state.messages[sessionKey] || []), characterMessage],
          },
          streamingText: { ...state.streamingText, [sessionKey]: '' },
          isStreaming: { ...state.isStreaming, [sessionKey]: false },
          cancelStream: { ...state.cancelStream, [sessionKey]: null },
          activeTurnId: { ...state.activeTurnId, [sessionKey]: undefined },
          activeTurn: { ...state.activeTurn, [sessionKey]: undefined },
          failedTurn: { ...state.failedTurn, [sessionKey]: undefined },
        };
      }),
      onError: error => set(state => {
        if (
          state.sessionGeneration !== generation
          || state.activeTurnId[sessionKey] !== turnId
        ) return state;
        return {
          streamingText: { ...state.streamingText, [sessionKey]: '' },
          isStreaming: { ...state.isStreaming, [sessionKey]: false },
          cancelStream: { ...state.cancelStream, [sessionKey]: null },
          activeTurnId: { ...state.activeTurnId, [sessionKey]: undefined },
          activeTurn: { ...state.activeTurn, [sessionKey]: undefined },
          failedTurn: { ...state.failedTurn, [sessionKey]: { ...turn, error } },
        };
      }),
    });

    set(state => {
      if (
        state.sessionGeneration !== generation
        || state.activeTurnId[sessionKey] !== turnId
        || !state.isStreaming[sessionKey]
      ) return state;
      return {
        cancelStream: { ...state.cancelStream, [sessionKey]: cancel },
      };
    });
  };

  return {
    messages: {},
    streamingText: {},
    isStreaming: {},
    cancelStream: {},
    activeTurnId: {},
    activeTurn: {},
    failedTurn: {},
    sessionGeneration: 0,

    addMessage: (sessionKey, message) => set(state => ({
      messages: {
        ...state.messages,
        [sessionKey]: [...(state.messages[sessionKey] || []), message],
      },
    })),

    clearMessages: sessionKey => set(state => ({
      messages: { ...state.messages, [sessionKey]: [] },
    })),

    reset: () => {
      Object.values(get().cancelStream).forEach(cancel => cancel?.());
      set(state => ({
        messages: {},
        streamingText: {},
        isStreaming: {},
        cancelStream: {},
        activeTurnId: {},
        activeTurn: {},
        failedTurn: {},
        sessionGeneration: state.sessionGeneration + 1,
      }));
    },

    sendMessage: ({
      characterId,
      novelId,
      message,
      readerIdentity,
      readerIdentityScope,
      currentChapter,
    }) => {
      const sessionKey = chatSessionKey(characterId, readerIdentityScope);
      const state = get();
      if (state.activeTurn[sessionKey] || state.failedTurn[sessionKey]) return false;
      const turn: ChatTurn = {
        turnId: crypto.randomUUID(),
        sessionKey,
        characterId,
        payload: {
          novel_id: novelId,
          message,
          reader_identity: readerIdentity,
          current_chapter: currentChapter,
        },
      };
      const userMessage: ChatMessage = {
        id: crypto.randomUUID(),
        turn_id: turn.turnId,
        role: 'user',
        content: message,
        character_id: characterId,
        chapter_context: currentChapter,
        created_at: new Date().toISOString(),
      };
      const generation = get().sessionGeneration;

      set(state => ({
        messages: {
          ...state.messages,
          [sessionKey]: [...(state.messages[sessionKey] || []), userMessage],
        },
        streamingText: { ...state.streamingText, [sessionKey]: '' },
        isStreaming: { ...state.isStreaming, [sessionKey]: true },
        cancelStream: { ...state.cancelStream, [sessionKey]: null },
        activeTurnId: { ...state.activeTurnId, [sessionKey]: turn.turnId },
        activeTurn: { ...state.activeTurn, [sessionKey]: turn },
        failedTurn: { ...state.failedTurn, [sessionKey]: undefined },
      }));
      startTurn(turn, generation);
      return true;
    },

    retryMessage: sessionKey => {
      const failed = get().failedTurn[sessionKey];
      if (!failed) return;
      get().cancelStream[sessionKey]?.();
      const turn: ChatTurn = {
        turnId: failed.turnId,
        sessionKey,
        characterId: failed.characterId,
        payload: failed.payload,
      };
      const generation = get().sessionGeneration;
      set(state => ({
        streamingText: { ...state.streamingText, [sessionKey]: '' },
        isStreaming: { ...state.isStreaming, [sessionKey]: true },
        cancelStream: { ...state.cancelStream, [sessionKey]: null },
        activeTurnId: { ...state.activeTurnId, [sessionKey]: turn.turnId },
        activeTurn: { ...state.activeTurn, [sessionKey]: turn },
        failedTurn: { ...state.failedTurn, [sessionKey]: undefined },
      }));
      startTurn(turn, generation);
    },

    cancelMessage: (sessionKey, expectedTurnId) => {
      const state = get();
      const turn = state.activeTurn[sessionKey];
      if (expectedTurnId && turn?.turnId !== expectedTurnId) return;
      state.cancelStream[sessionKey]?.();
      set(current => ({
        streamingText: { ...current.streamingText, [sessionKey]: '' },
        isStreaming: { ...current.isStreaming, [sessionKey]: false },
        cancelStream: { ...current.cancelStream, [sessionKey]: null },
        activeTurnId: { ...current.activeTurnId, [sessionKey]: undefined },
        activeTurn: { ...current.activeTurn, [sessionKey]: undefined },
        failedTurn: {
          ...current.failedTurn,
          [sessionKey]: turn ? {
            ...turn,
            error: { code: 'cancelled', message: '已停止接收，可重试此消息。' },
          } : current.failedTurn[sessionKey],
        },
      }));
    },
  };
});
