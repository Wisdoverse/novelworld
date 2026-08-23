import React, { useState, useRef, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Send, X, Minimize2, Maximize2, Brain } from 'lucide-react';
import ReactMarkdown, { type Components } from 'react-markdown';
import { useChatHistory } from '@/features/character-chat/api/useChatHistory';
import { useChatStore } from '@/features/character-chat/model/useChatStore';
import type { Character, ChatMessage } from '@/shared/types';

const EMPTY_MESSAGES: ChatMessage[] = [];
const SAFE_MARKDOWN_COMPONENTS: Components = {
  img: ({ alt }) => <span>{alt}</span>,
};

export function ChatMarkdown({ children }: { children: string }) {
  return <ReactMarkdown components={SAFE_MARKDOWN_COMPONENTS}>{children}</ReactMarkdown>;
}

export function mergeVisibleChatMessages(
  history: ChatMessage[],
  session: ChatMessage[],
  currentChapter: number,
) {
  const seen = new Set<string>();
  return [...history, ...session].filter(message => {
    if (message.chapter_context != null && message.chapter_context > currentChapter) return false;
    const key = message.turn_id ? `${message.turn_id}:${message.role}` : message.id;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

interface ChatPanelProps {
  character: Character;
  novelId: string;
  currentChapter: number;
  readerIdentity?: string;
  canChat: boolean;
  isOpen: boolean;
  onClose: () => void;
}

export function ChatPanel({
  character,
  novelId,
  currentChapter,
  readerIdentity,
  canChat,
  isOpen,
  onClose,
}: ChatPanelProps) {
  const [input, setInput] = useState('');
  const [isMinimized, setIsMinimized] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  const sessionMessages = useChatStore(state => state.messages[character.id]) ?? EMPTY_MESSAGES;
  const rawStreamText = useChatStore(state => state.streamingText[character.id]) || '';
  const isCharacterStreaming = useChatStore(state => state.isStreaming[character.id]) || false;
  const activeTurn = useChatStore(state => state.activeTurn[character.id]);
  const failedTurn = useChatStore(state => state.failedTurn[character.id]);
  const sendMessage = useChatStore(state => state.sendMessage);
  const retryMessage = useChatStore(state => state.retryMessage);
  const cancelMessage = useChatStore(state => state.cancelMessage);
  const characterMatchesNovel = character.novel_id === novelId;
  const history = useChatHistory(
    character.id,
    currentChapter,
    isOpen && canChat && characterMatchesNovel,
  );
  const activeTurnMatchesView = activeTurn?.payload.novel_id === novelId
    && activeTurn.payload.current_chapter === currentChapter;
  const isCurrentlyStreaming = isCharacterStreaming && activeTurnMatchesView;
  const currentStreamText = isCurrentlyStreaming ? rawStreamText : '';
  const visibleFailedTurn = failedTurn?.payload.novel_id === novelId
    && failedTurn.payload.current_chapter === currentChapter
    ? failedTurn
    : undefined;
  const charMessages = characterMatchesNovel
    ? mergeVisibleChatMessages(
      history.data ?? EMPTY_MESSAGES,
      sessionMessages,
      currentChapter,
    )
    : EMPTY_MESSAGES;
  const historyReady = !history.isLoading && !history.isError;

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView?.({ behavior: 'smooth' });
  }, [charMessages, currentStreamText]);

  useEffect(() => {
    if (activeTurn && !activeTurnMatchesView) {
      cancelMessage(character.id, activeTurn.turnId);
    }
  }, [activeTurn, activeTurnMatchesView, cancelMessage, character.id]);

  const handleSend = () => {
    if (!canChat || !historyReady || !input.trim() || isCurrentlyStreaming) return;
    sendMessage({
      characterId: character.id,
      novelId,
      message: input.trim(),
      readerIdentity,
      currentChapter,
    });
    setInput('');
    inputRef.current?.focus();
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  return (
    <AnimatePresence>
      {isOpen && characterMatchesNovel && (
        <motion.div
          initial={{ opacity: 0, x: 60, scale: 0.95 }}
          animate={{ opacity: 1, x: 0, scale: 1 }}
          exit={{ opacity: 0, x: 60, scale: 0.95 }}
          transition={{ duration: 0.25, ease: [0.23, 1, 0.32, 1] }}
          className="fixed right-4 bottom-4 z-50 flex flex-col"
          style={{
            width: 'min(380px, calc(100vw - 2rem))',
            height: isMinimized ? '64px' : '560px',
            transition: 'height 250ms cubic-bezier(0.23, 1, 0.32, 1)',
          }}
        >
          <div className="surface-card flex h-full flex-col overflow-hidden">
            {/* Header */}
            <div
              className="flex cursor-pointer items-center gap-3 border-b border-[#e1e3e8] p-4"
              onClick={() => setIsMinimized(!isMinimized)}
            >
              {/* 角色头像 */}
              <div className="relative flex-shrink-0">
                {character.avatar_url ? (
                  <img
                    src={character.avatar_url}
                    alt={character.name}
                    className="w-10 h-10 rounded-full object-cover"
                    style={{ border: '2px solid #d2e3fc' }}
                  />
                ) : (
                  <div
                    className="w-10 h-10 rounded-full flex items-center justify-center text-lg font-bold"
                    style={{
                      background: '#0b57d0',
                      border: '2px solid #d2e3fc',
                      color: 'white',
                    }}
                  >
                    {character.name[0]}
                  </div>
                )}
                {/* 在线指示器 */}
                <div
                  className="absolute -bottom-0.5 -right-0.5 h-3 w-3 animate-pulse rounded-full"
                  style={{ background: '#1e8e3e' }}
                />
              </div>

              <div className="flex-1 min-w-0">
                <div className="truncate text-sm font-semibold text-[#1f1f1f]">
                  {character.name}
                </div>
                <div className="truncate text-xs text-[#5f6368]">
                  {isCurrentlyStreaming ? (
                    <span className="text-[#0b57d0]" style={{ fontSize: '11px' }}>正在思考...</span>
                  ) : (
                    <span>{character.role === 'protagonist' ? '主角' : '角色'}</span>
                  )}
                </div>
              </div>

              <div className="flex items-center gap-1">
                <button
                  onClick={(e) => { e.stopPropagation(); setIsMinimized(!isMinimized); }}
                  aria-label={isMinimized ? '展开聊天窗口' : '收起聊天窗口'}
                  className="min-h-8 min-w-8 rounded-full p-1.5 text-[#5f6368] transition-colors hover:bg-[#f1f3f4]"
                >
                  {isMinimized ? <Maximize2 size={14} /> : <Minimize2 size={14} />}
                </button>
                <button
                  onClick={(e) => { e.stopPropagation(); onClose(); }}
                  aria-label="关闭聊天"
                  className="min-h-8 min-w-8 rounded-full p-1.5 text-[#5f6368] transition-colors hover:bg-[#f1f3f4]"
                >
                  <X size={14} />
                </button>
              </div>
            </div>

            {/* Messages */}
            {!isMinimized && (
              <>
                <div
                  className="flex-1 overflow-y-auto p-4 space-y-4"
                  style={{ minHeight: 0 }}
                >
                  {history.isLoading ? (
                    <p role="status" className="py-8 text-center text-sm text-[#5f6368]">
                      正在恢复对话记录…
                    </p>
                  ) : null}
                  {history.isError ? (
                    <div role="alert" className="py-8 text-center text-sm text-[#b3261e]">
                      <p>对话记录加载失败，暂时不能继续发送。</p>
                      <button type="button" className="mt-2 underline" onClick={() => void history.refetch()}>
                        重试加载
                      </button>
                    </div>
                  ) : null}
                  {historyReady && charMessages.length === 0 && (
                    <div className="text-center py-8">
                      <Brain size={32} className="mx-auto mb-3 text-[#0b57d0] opacity-40" />
                      <p className="text-sm text-[#5f6368]">
                        与 {character.name} 开始对话
                      </p>
                      <p className="mt-1 text-xs text-[#5f6368]">
                        会参考已提交的对话与可用记忆
                      </p>
                    </div>
                  )}

                  {charMessages.map((msg) => (
                    <div
                      key={msg.id}
                      className={`flex ${msg.role === 'user' ? 'justify-end' : 'justify-start'} gap-2`}
                    >
                      {msg.role === 'character' && (
                        <div
                          className="w-7 h-7 rounded-full flex-shrink-0 flex items-center justify-center text-xs font-bold mt-1"
                          style={{ background: '#0b57d0', color: 'white' }}
                        >
                          {character.name[0]}
                        </div>
                      )}
                      <div
                        className={`max-w-[80%] text-sm leading-relaxed ${
                          msg.role === 'user' ? 'chat-bubble-user' : 'chat-bubble-character'
                        }`}
                      >
                        <ChatMarkdown>{msg.content}</ChatMarkdown>
                      </div>
                    </div>
                  ))}

                  {/* 流式输出 */}
                  {isCurrentlyStreaming && currentStreamText && (
                    <div className="flex justify-start gap-2">
                      <div
                        className="w-7 h-7 rounded-full flex-shrink-0 flex items-center justify-center text-xs font-bold mt-1 animate-pulse"
                        style={{ background: '#0b57d0', color: 'white' }}
                      >
                        {character.name[0]}
                      </div>
                      <div className="chat-bubble-character max-w-[80%] text-sm leading-relaxed">
                        <ChatMarkdown>{currentStreamText}</ChatMarkdown>
                        <span className="ml-0.5 inline-block h-4 w-1 animate-pulse bg-[#0b57d0]" />
                      </div>
                    </div>
                  )}

                  <div ref={messagesEndRef} />
                </div>

                {/* Input */}
                <div className="border-t border-[#e1e3e8] p-3">
                  {readerIdentity && (
                    <div className="mb-2 rounded bg-[#e8f0fe] px-2 py-1 text-xs text-[#0b57d0]">
                      以「{readerIdentity}」身份对话
                    </div>
                  )}
                  {visibleFailedTurn && !isCurrentlyStreaming && (
                    <div
                      className="mb-2 flex items-center justify-between gap-2 rounded bg-[#fce8e6] px-2 py-1.5 text-xs text-[#b3261e]"
                      role="alert"
                    >
                      <span>{visibleFailedTurn.error.message}</span>
                      <button
                        type="button"
                        className="shrink-0 underline disabled:opacity-50"
                        disabled={!canChat || !historyReady}
                        onClick={() => retryMessage(character.id)}
                      >
                        重试
                      </button>
                    </div>
                  )}
                  <div className="flex gap-2 items-end">
                    <textarea
                      ref={inputRef}
                      value={input}
                      onChange={(e) => setInput(e.target.value)}
                      onKeyDown={handleKeyDown}
                      placeholder={`对 ${character.name} 说...`}
                      rows={2}
                      maxLength={4000}
                      className="field-control flex-1 resize-none text-sm"
                      style={{ fontFamily: 'var(--font-body)' }}
                      disabled={!canChat || !historyReady || isCurrentlyStreaming}
                    />
                    {isCurrentlyStreaming && (
                      <button
                        type="button"
                        onClick={() => cancelMessage(character.id)}
                        className="flex-shrink-0 rounded-full bg-[#fce8e6] px-3 py-2.5 text-xs font-semibold text-[#b3261e] transition-colors hover:bg-[#f9dedc]"
                      >
                        停止
                      </button>
                    )}
                    <button
                      type="button"
                      onClick={handleSend}
                      aria-label="发送消息"
                      disabled={!canChat || !historyReady || !input.trim() || isCurrentlyStreaming}
                      className="flex-shrink-0 rounded-full bg-[#0b57d0] p-2.5 text-white transition-colors hover:bg-[#0842a0] disabled:cursor-not-allowed disabled:bg-[#c4c7c5] disabled:text-[#747775]"
                    >
                      <Send size={16} />
                    </button>
                  </div>
                </div>
              </>
            )}
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
