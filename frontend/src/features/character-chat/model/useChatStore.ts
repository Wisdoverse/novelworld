import { create } from 'zustand';
import { createChatStream } from '@/shared/api/client';
import type { ChatMessage } from '@/shared/types';

interface ChatState {
  messages: Record<string, ChatMessage[]>; // key: characterId
  streamingText: Record<string, string>;   // key: characterId
  isStreaming: Record<string, boolean>;
  cancelStream: Record<string, (() => void) | null>;

  sendMessage: (params: {
    characterId: string;
    userId: string;
    novelId: string;
    message: string;
    readerIdentity?: string;
    currentChapter: number;
  }) => void;

  addMessage: (characterId: string, message: ChatMessage) => void;
  clearMessages: (characterId: string) => void;
}

export const useChatStore = create<ChatState>((set, get) => ({
  messages: {},
  streamingText: {},
  isStreaming: {},
  cancelStream: {},

  addMessage: (characterId, message) => {
    set((state) => ({
      messages: {
        ...state.messages,
        [characterId]: [...(state.messages[characterId] || []), message],
      },
    }));
  },

  clearMessages: (characterId) => {
    set((state) => ({
      messages: { ...state.messages, [characterId]: [] },
    }));
  },

  sendMessage: ({ characterId, userId, novelId, message, readerIdentity, currentChapter }) => {
    const { isStreaming, cancelStream } = get();

    // 取消正在进行的流
    if (isStreaming[characterId] && cancelStream[characterId]) {
      cancelStream[characterId]?.();
    }

    // 添加用户消息
    const userMsg: ChatMessage = {
      id: crypto.randomUUID(),
      role: 'user',
      content: message,
      character_id: characterId,
      created_at: new Date().toISOString(),
    };
    get().addMessage(characterId, userMsg);

    // 初始化流式状态
    set((state) => ({
      streamingText: { ...state.streamingText, [characterId]: '' },
      isStreaming: { ...state.isStreaming, [characterId]: true },
    }));

    // 启动 SSE 流
    const cancel = createChatStream(
      characterId,
      {
        user_id: userId,
        novel_id: novelId,
        message,
        reader_identity: readerIdentity,
        current_chapter: currentChapter,
      },
      // onChunk
      (chunk) => {
        set((state) => ({
          streamingText: {
            ...state.streamingText,
            [characterId]: (state.streamingText[characterId] || '') + chunk,
          },
        }));
      },
      // onDone
      () => {
        const finalText = get().streamingText[characterId] || '';
        const charMsg: ChatMessage = {
          id: crypto.randomUUID(),
          role: 'character',
          content: finalText,
          character_id: characterId,
          created_at: new Date().toISOString(),
        };
        get().addMessage(characterId, charMsg);
        set((state) => ({
          streamingText: { ...state.streamingText, [characterId]: '' },
          isStreaming: { ...state.isStreaming, [characterId]: false },
          cancelStream: { ...state.cancelStream, [characterId]: null },
        }));
      },
      // onError
      (err) => {
        console.error('Chat stream error:', err);
        set((state) => ({
          streamingText: { ...state.streamingText, [characterId]: '' },
          isStreaming: { ...state.isStreaming, [characterId]: false },
        }));
      },
    );

    set((state) => ({
      cancelStream: { ...state.cancelStream, [characterId]: cancel },
    }));
  },
}));
