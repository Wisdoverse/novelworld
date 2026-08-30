import { useEffect, useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { CheckCircle2, GitBranch, Sparkles, ChevronRight } from 'lucide-react';
import type { NarrativeNode, NarrativeChoice } from '@/shared/types';

interface BranchChoiceProps {
  node: NarrativeNode;
  onChoose: (choice: NarrativeChoice) => Promise<void>;
  isLoading?: boolean;
  selectedChoiceIndex?: number;
  consequence?: string;
  error?: string;
  isRecoveryLocked?: boolean;
  onRetryRecovery?: () => Promise<void>;
}

export function BranchChoice({
  node,
  onChoose,
  isLoading = false,
  selectedChoiceIndex,
  consequence,
  error,
  isRecoveryLocked = false,
  onRetryRecovery,
}: BranchChoiceProps) {
  const [pendingIndex, setPendingIndex] = useState<number | null>(null);
  const [hoveredIndex, setHoveredIndex] = useState<number | null>(null);
  const selectedIndex = selectedChoiceIndex ?? pendingIndex;

  useEffect(() => {
    if (!isLoading && error) setPendingIndex(null);
  }, [error, isLoading]);

  const handleChoose = async (choice: NarrativeChoice) => {
    if (selectedChoiceIndex !== undefined || pendingIndex !== null || isLoading || isRecoveryLocked) return;
    setPendingIndex(choice.index);
    try {
      await onChoose(choice);
    } catch {
      setPendingIndex(null);
    }
  };

  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.4, ease: [0.23, 1, 0.32, 1] }}
      className="my-8 mx-auto max-w-2xl"
    >
      {/* 标题 */}
      <div className="flex items-center gap-3 mb-6">
        <div className="flex h-10 w-10 items-center justify-center rounded-full bg-[#e8f0fe] text-[#0b57d0]">
          <GitBranch size={18} />
        </div>
        <div>
          <div className="mb-1 text-xs font-semibold uppercase tracking-widest text-[#0b57d0]">
            命运交叉点
          </div>
          <p className="text-sm leading-relaxed text-[#5f6368]">
            {node.description}
          </p>
        </div>
      </div>

      {/* 选项列表 */}
      <div className="space-y-3">
        {node.choices.map((choice, i) => (
          <motion.button
            key={choice.index}
            initial={{ opacity: 0, x: -20 }}
            animate={{ opacity: 1, x: 0 }}
            transition={{ delay: i * 0.1, duration: 0.3, ease: [0.23, 1, 0.32, 1] }}
            onClick={() => handleChoose(choice)}
            onMouseEnter={() => setHoveredIndex(i)}
            onMouseLeave={() => setHoveredIndex(null)}
            aria-pressed={selectedChoiceIndex === choice.index}
            disabled={selectedChoiceIndex !== undefined || pendingIndex !== null || isLoading || isRecoveryLocked}
            className="choice-card w-full text-left"
            style={{
              opacity: selectedIndex !== null && selectedIndex !== choice.index ? 0.4 : 1,
              background: selectedIndex === choice.index
                ? '#e8f0fe'
                : undefined,
              borderColor: selectedIndex === choice.index
                ? '#0b57d0'
                : undefined,
            }}
          >
            <div className="flex items-start gap-3">
              {/* 选项序号 */}
              <div
                className="flex-shrink-0 w-7 h-7 rounded-full flex items-center justify-center text-xs font-bold mt-0.5"
                style={{
                  background: hoveredIndex === i || selectedIndex === choice.index
                    ? '#0b57d0'
                    : '#5f6368',
                  color: 'white',
                  transition: 'background 200ms',
                }}
              >
                {String.fromCharCode(65 + i)}
              </div>

              <div className="flex-1 min-w-0">
                <p className="text-sm leading-relaxed text-[#1f1f1f]">
                  {choice.text}
                  {selectedChoiceIndex === choice.index ? <span className="sr-only">（已选择）</span> : null}
                </p>
                {choice.hint && (
                  <p className="mt-1.5 flex items-center gap-1 text-xs text-[#0b57d0]">
                    <Sparkles size={10} />
                    {choice.hint}
                  </p>
                )}
              </div>

              <ChevronRight
                size={16}
                className="flex-shrink-0 mt-0.5 transition-transform"
                style={{
                  color: '#5f6368',
                  transform: hoveredIndex === i ? 'translateX(3px)' : 'none',
                }}
              />
            </div>
          </motion.button>
        ))}
      </div>

      {/* 加载状态 */}
      <AnimatePresence>
        {isLoading && selectedIndex !== null && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: 'auto' }}
            exit={{ opacity: 0, height: 0 }}
            className="mt-4 rounded-xl border border-[#d2e3fc] bg-[#f8faff] p-4 text-center"
          >
            <div className="flex items-center justify-center gap-2 text-sm text-[#0b57d0]">
              <div className="h-4 w-4 animate-spin rounded-full border-2 border-[#0b57d0] border-t-transparent" />
              正在根据你的行动重新生成后续内容...
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {error && !isLoading && (
        <div
          role="alert"
          className="mt-4 rounded-xl border border-[#f2b8b5] bg-[#fce8e6] p-4 text-sm text-[#b3261e]"
        >
          {error}
          {isRecoveryLocked && onRetryRecovery ? (
            <button
              type="button"
              className="ml-2 underline"
              onClick={() => void onRetryRecovery()}
            >
              重新加载已提交结果
            </button>
          ) : null}
        </div>
      )}

      {consequence && selectedChoiceIndex !== undefined && (
        <motion.div
          initial={{ opacity: 0, y: 12 }}
          animate={{ opacity: 1, y: 0 }}
          className="mt-6 rounded-xl border border-[#a8dab5] bg-[#e6f4ea] p-5"
        >
          <div className="mb-3 flex items-center gap-2 text-sm font-semibold text-[#0d652d]">
            <CheckCircle2 size={16} />
            你的行动改变了后续故事
          </div>
          <p className="whitespace-pre-wrap text-sm leading-7 text-[#1f1f1f]">
            {consequence}
          </p>
        </motion.div>
      )}
    </motion.div>
  );
}
