import React, { useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { GitBranch, Sparkles, ChevronRight } from 'lucide-react';
import type { NarrativeNode, NarrativeChoice } from '@/shared/types';

interface BranchChoiceProps {
  node: NarrativeNode;
  onChoose: (choice: NarrativeChoice) => void;
  isLoading?: boolean;
}

export function BranchChoice({ node, onChoose, isLoading }: BranchChoiceProps) {
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);
  const [hoveredIndex, setHoveredIndex] = useState<number | null>(null);

  const handleChoose = (choice: NarrativeChoice) => {
    if (selectedIndex !== null || isLoading) return;
    setSelectedIndex(choice.index);
    onChoose(choice);
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
        <div
          className="w-10 h-10 rounded-full flex items-center justify-center"
          style={{ background: 'linear-gradient(135deg, rgba(109,40,217,0.3), rgba(6,182,212,0.3))', border: '1px solid rgba(6,182,212,0.3)' }}
        >
          <GitBranch size={18} style={{ color: '#22d3ee' }} />
        </div>
        <div>
          <div className="text-xs font-semibold uppercase tracking-widest mb-1" style={{ color: '#6d28d9' }}>
            命运交叉点
          </div>
          <p className="text-sm leading-relaxed" style={{ color: '#94a3b8' }}>
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
            disabled={selectedIndex !== null || isLoading}
            className="choice-card w-full text-left"
            style={{
              opacity: selectedIndex !== null && selectedIndex !== choice.index ? 0.4 : 1,
              background: selectedIndex === choice.index
                ? 'rgba(6, 182, 212, 0.15)'
                : undefined,
              borderColor: selectedIndex === choice.index
                ? 'rgba(6, 182, 212, 0.6)'
                : undefined,
            }}
          >
            <div className="flex items-start gap-3">
              {/* 选项序号 */}
              <div
                className="flex-shrink-0 w-7 h-7 rounded-full flex items-center justify-center text-xs font-bold mt-0.5"
                style={{
                  background: hoveredIndex === i || selectedIndex === choice.index
                    ? 'linear-gradient(135deg, #0891b2, #6d28d9)'
                    : 'rgba(71, 85, 105, 0.4)',
                  color: 'white',
                  transition: 'background 200ms',
                }}
              >
                {String.fromCharCode(65 + i)}
              </div>

              <div className="flex-1 min-w-0">
                <p className="text-sm leading-relaxed" style={{ color: '#e2e8f0' }}>
                  {choice.text}
                </p>
                {choice.hint && (
                  <p className="text-xs mt-1.5 flex items-center gap-1" style={{ color: '#6d28d9' }}>
                    <Sparkles size={10} />
                    {choice.hint}
                  </p>
                )}
              </div>

              <ChevronRight
                size={16}
                className="flex-shrink-0 mt-0.5 transition-transform"
                style={{
                  color: '#475569',
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
            className="mt-4 p-4 rounded-xl text-center"
            style={{ background: 'rgba(6, 182, 212, 0.08)', border: '1px solid rgba(6, 182, 212, 0.2)' }}
          >
            <div className="flex items-center justify-center gap-2 text-sm" style={{ color: '#22d3ee' }}>
              <div className="w-4 h-4 border-2 rounded-full animate-spin" style={{ borderColor: '#22d3ee', borderTopColor: 'transparent' }} />
              命运正在重写...
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </motion.div>
  );
}
