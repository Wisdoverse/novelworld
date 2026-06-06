import React, { useState, useRef, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Send, X, Minimize2, Maximize2, Brain } from 'lucide-react';
import ReactMarkdown from 'react-markdown';
import { useChatStore } from '@/features/character-chat/model/useChatStore';
import type { Character } from '@/shared/types';

interface ChatPanelProps {
  character: Character;
  userId: string;
  novelId: string;
  currentChapter: number;
  readerIdentity?: string;
  isOpen: boolean;
  onClose: () => void;
}

export function ChatPanel({
  character,
  userId,
  novelId,
  currentChapter,
  readerIdentity,
  isOpen,
  onClose,
}: ChatPanelProps) {
  const [input, setInput] = useState('');
  const [isMinimized, setIsMinimized] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  const { messages, streamingText, isStreaming, sendMessage } = useChatStore();
  const charMessages = messages[character.id] || [];
  const currentStreamText = streamingText[character.id] || '';
  const isCurrentlyStreaming = isStreaming[character.id] || false;

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [charMessages, currentStreamText]);

  const handleSend = () => {
    if (!input.trim() || isCurrentlyStreaming) return;
    sendMessage({
      characterId: character.id,
      userId,
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
      {isOpen && (
        <motion.div
          initial={{ opacity: 0, x: 60, scale: 0.95 }}
          animate={{ opacity: 1, x: 0, scale: 1 }}
          exit={{ opacity: 0, x: 60, scale: 0.95 }}
          transition={{ duration: 0.25, ease: [0.23, 1, 0.32, 1] }}
          className="fixed right-4 bottom-4 z-50 flex flex-col"
          style={{
            width: '380px',
            height: isMinimized ? '64px' : '560px',
            transition: 'height 250ms cubic-bezier(0.23, 1, 0.32, 1)',
          }}
        >
          <div className="glass-card flex flex-col h-full overflow-hidden">
            {/* Header */}
            <div
              className="flex items-center gap-3 p-4 border-b cursor-pointer"
              style={{ borderColor: 'rgba(109, 40, 217, 0.2)' }}
              onClick={() => setIsMinimized(!isMinimized)}
            >
              {/* 角色头像 */}
              <div className="relative flex-shrink-0">
                {character.avatar_url ? (
                  <img
                    src={character.avatar_url}
                    alt={character.name}
                    className="w-10 h-10 rounded-full object-cover"
                    style={{ border: '2px solid rgba(6, 182, 212, 0.4)' }}
                  />
                ) : (
                  <div
                    className="w-10 h-10 rounded-full flex items-center justify-center text-lg font-bold"
                    style={{
                      background: 'linear-gradient(135deg, #6d28d9, #06b6d4)',
                      border: '2px solid rgba(6, 182, 212, 0.4)',
                    }}
                  >
                    {character.name[0]}
                  </div>
                )}
                {/* 在线指示器 */}
                <div
                  className="absolute -bottom-0.5 -right-0.5 w-3 h-3 rounded-full animate-pulse-glow"
                  style={{ background: '#22d3ee' }}
                />
              </div>

              <div className="flex-1 min-w-0">
                <div className="font-semibold text-sm truncate" style={{ color: '#e2e8f0' }}>
                  {character.name}
                </div>
                <div className="text-xs truncate" style={{ color: '#94a3b8' }}>
                  {isCurrentlyStreaming ? (
                    <span className="text-glow" style={{ fontSize: '11px' }}>正在思考...</span>
                  ) : (
                    <span>{character.role === 'protagonist' ? '主角' : '角色'}</span>
                  )}
                </div>
              </div>

              <div className="flex items-center gap-1">
                <button
                  onClick={(e) => { e.stopPropagation(); setIsMinimized(!isMinimized); }}
                  className="p-1.5 rounded-lg transition-colors hover:bg-white/10"
                  style={{ color: '#94a3b8' }}
                >
                  {isMinimized ? <Maximize2 size={14} /> : <Minimize2 size={14} />}
                </button>
                <button
                  onClick={(e) => { e.stopPropagation(); onClose(); }}
                  className="p-1.5 rounded-lg transition-colors hover:bg-white/10"
                  style={{ color: '#94a3b8' }}
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
                  {charMessages.length === 0 && (
                    <div className="text-center py-8">
                      <Brain size={32} className="mx-auto mb-3 opacity-30" style={{ color: '#6d28d9' }} />
                      <p className="text-sm" style={{ color: '#475569' }}>
                        与 {character.name} 开始对话
                      </p>
                      <p className="text-xs mt-1" style={{ color: '#334155' }}>
                        TA 记得你们之前的所有互动
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
                          style={{ background: 'linear-gradient(135deg, #6d28d9, #06b6d4)' }}
                        >
                          {character.name[0]}
                        </div>
                      )}
                      <div
                        className={`max-w-[80%] text-sm leading-relaxed ${
                          msg.role === 'user' ? 'chat-bubble-user' : 'chat-bubble-character'
                        }`}
                      >
                        <ReactMarkdown>{msg.content}</ReactMarkdown>
                      </div>
                    </div>
                  ))}

                  {/* 流式输出 */}
                  {isCurrentlyStreaming && currentStreamText && (
                    <div className="flex justify-start gap-2">
                      <div
                        className="w-7 h-7 rounded-full flex-shrink-0 flex items-center justify-center text-xs font-bold mt-1 animate-pulse"
                        style={{ background: 'linear-gradient(135deg, #6d28d9, #06b6d4)' }}
                      >
                        {character.name[0]}
                      </div>
                      <div className="chat-bubble-character max-w-[80%] text-sm leading-relaxed">
                        <ReactMarkdown>{currentStreamText}</ReactMarkdown>
                        <span className="inline-block w-1 h-4 ml-0.5 animate-pulse" style={{ background: '#22d3ee' }} />
                      </div>
                    </div>
                  )}

                  <div ref={messagesEndRef} />
                </div>

                {/* Input */}
                <div className="p-3 border-t" style={{ borderColor: 'rgba(109, 40, 217, 0.2)' }}>
                  {readerIdentity && (
                    <div className="mb-2 px-2 py-1 rounded text-xs" style={{ background: 'rgba(6, 182, 212, 0.1)', color: '#22d3ee' }}>
                      以「{readerIdentity}」身份对话
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
                      className="flex-1 resize-none rounded-xl px-3 py-2 text-sm outline-none"
                      style={{
                        background: 'rgba(15, 21, 53, 0.8)',
                        border: '1px solid rgba(109, 40, 217, 0.3)',
                        color: '#e2e8f0',
                        fontFamily: 'var(--font-body)',
                      }}
                      disabled={isCurrentlyStreaming}
                    />
                    <button
                      onClick={handleSend}
                      disabled={!input.trim() || isCurrentlyStreaming}
                      className="flex-shrink-0 p-2.5 rounded-xl transition-all"
                      style={{
                        background: input.trim() && !isCurrentlyStreaming
                          ? 'linear-gradient(135deg, #0891b2, #6d28d9)'
                          : 'rgba(71, 85, 105, 0.3)',
                        color: input.trim() && !isCurrentlyStreaming ? 'white' : '#475569',
                        cursor: input.trim() && !isCurrentlyStreaming ? 'pointer' : 'not-allowed',
                      }}
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
