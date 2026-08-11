import { create } from 'zustand';

import {
  createChatStream,
  type ChatStreamError,
  type ChatStreamPayload,
} from '@/shared/api/client';
import type { ChatMessage } from '@/shared/types';

interface ChatTurn {
  turnId: string;
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
    currentChapter: number;
  }) => void;
  retryMessage: (characterId: string) => void;
  cancelMessage: (characterId: string) => void;
  addMessage: (characterId: string, message: ChatMessage) => void;
  clearMessages: (characterId: string) => void;
  reset: () => void;
}

export const useChatStore = create<ChatState>((set, get) => {
  const startTurn = (turn: ChatTurn, generation: number) => {
    const { characterId, turnId, payload } = turn;
    const cancel = createChatStream({
      characterId,
      turnId,
      payload,
      onChunk: chunk => set(state => {
        if (
          state.sessionGeneration !== generation
          || state.activeTurnId[characterId] !== turnId
        ) return state;
        return {
          streamingText: {
            ...state.streamingText,
            [characterId]: (state.streamingText[characterId] || '') + chunk,
          },
        };
      }),
      onRetry: () => set(state => {
        if (
          state.sessionGeneration !== generation
          || state.activeTurnId[characterId] !== turnId
        ) return state;
        return {
          streamingText: { ...state.streamingText, [characterId]: '' },
        };
      }),
      onDone: result => set(state => {
        if (
          result.turnId !== turnId
          || state.sessionGeneration !== generation
          || state.activeTurnId[characterId] !== turnId
        ) return state;
        const finalText = state.streamingText[characterId] || '';
        const characterMessage: ChatMessage = {
          id: crypto.randomUUID(),
          role: 'character',
          content: finalText,
          character_id: characterId,
          created_at: new Date().toISOString(),
        };
        return {
          messages: {
            ...state.messages,
            [characterId]: [...(state.messages[characterId] || []), characterMessage],
          },
          streamingText: { ...state.streamingText, [characterId]: '' },
          isStreaming: { ...state.isStreaming, [characterId]: false },
          cancelStream: { ...state.cancelStream, [characterId]: null },
          activeTurnId: { ...state.activeTurnId, [characterId]: undefined },
          activeTurn: { ...state.activeTurn, [characterId]: undefined },
          failedTurn: { ...state.failedTurn, [characterId]: undefined },
        };
      }),
      onError: error => set(state => {
        if (
          state.sessionGeneration !== generation
          || state.activeTurnId[characterId] !== turnId
        ) return state;
        return {
          streamingText: { ...state.streamingText, [characterId]: '' },
          isStreaming: { ...state.isStreaming, [characterId]: false },
          cancelStream: { ...state.cancelStream, [characterId]: null },
          activeTurn: { ...state.activeTurn, [characterId]: undefined },
          failedTurn: { ...state.failedTurn, [characterId]: { ...turn, error } },
        };
      }),
    });

    set(state => {
      if (
        state.sessionGeneration !== generation
        || state.activeTurnId[characterId] !== turnId
        || !state.isStreaming[characterId]
      ) return state;
      return {
        cancelStream: { ...state.cancelStream, [characterId]: cancel },
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

    addMessage: (characterId, message) => set(state => ({
      messages: {
        ...state.messages,
        [characterId]: [...(state.messages[characterId] || []), message],
      },
    })),

    clearMessages: characterId => set(state => ({
      messages: { ...state.messages, [characterId]: [] },
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

    sendMessage: ({ characterId, novelId, message, readerIdentity, currentChapter }) => {
      get().cancelStream[characterId]?.();
      const turn: ChatTurn = {
        turnId: crypto.randomUUID(),
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
        role: 'user',
        content: message,
        character_id: characterId,
        created_at: new Date().toISOString(),
      };
      const generation = get().sessionGeneration;

      set(state => ({
        messages: {
          ...state.messages,
          [characterId]: [...(state.messages[characterId] || []), userMessage],
        },
        streamingText: { ...state.streamingText, [characterId]: '' },
        isStreaming: { ...state.isStreaming, [characterId]: true },
        cancelStream: { ...state.cancelStream, [characterId]: null },
        activeTurnId: { ...state.activeTurnId, [characterId]: turn.turnId },
        activeTurn: { ...state.activeTurn, [characterId]: turn },
        failedTurn: { ...state.failedTurn, [characterId]: undefined },
      }));
      startTurn(turn, generation);
    },

    retryMessage: characterId => {
      const failed = get().failedTurn[characterId];
      if (!failed) return;
      get().cancelStream[characterId]?.();
      const turn: ChatTurn = {
        turnId: failed.turnId,
        characterId: failed.characterId,
        payload: failed.payload,
      };
      const generation = get().sessionGeneration;
      set(state => ({
        streamingText: { ...state.streamingText, [characterId]: '' },
        isStreaming: { ...state.isStreaming, [characterId]: true },
        cancelStream: { ...state.cancelStream, [characterId]: null },
        activeTurnId: { ...state.activeTurnId, [characterId]: turn.turnId },
        activeTurn: { ...state.activeTurn, [characterId]: turn },
        failedTurn: { ...state.failedTurn, [characterId]: undefined },
      }));
      startTurn(turn, generation);
    },

    cancelMessage: characterId => {
      const state = get();
      const turn = state.activeTurn[characterId];
      state.cancelStream[characterId]?.();
      set(current => ({
        streamingText: { ...current.streamingText, [characterId]: '' },
        isStreaming: { ...current.isStreaming, [characterId]: false },
        cancelStream: { ...current.cancelStream, [characterId]: null },
        activeTurnId: { ...current.activeTurnId, [characterId]: undefined },
        activeTurn: { ...current.activeTurn, [characterId]: undefined },
        failedTurn: {
          ...current.failedTurn,
          [characterId]: turn ? {
            ...turn,
            error: { code: 'cancelled', message: '已停止接收，可重试此消息。' },
          } : current.failedTurn[characterId],
        },
      }));
    },
  };
});
