import React, { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { motion, AnimatePresence } from 'framer-motion';
import { Plus, BookOpen, Clock, Trash2, Loader2, CheckCircle, AlertCircle, Upload } from 'lucide-react';
import { useNovels, useImportNovel, useDeleteNovel } from '@/entities/novel/api';
import type { Novel } from '@/shared/types';

function NovelCard({ novel, onOpen, onDelete }: {
  novel: Novel;
  onOpen: () => void;
  onDelete: () => void;
}) {
  const statusConfig = {
    pending: { icon: Loader2, color: '#94a3b8', label: '等待解析', spin: true },
    parsing: { icon: Loader2, color: '#22d3ee', label: '解析中...', spin: true },
    ready: { icon: CheckCircle, color: '#22c55e', label: '已就绪', spin: false },
    error: { icon: AlertCircle, color: '#ef4444', label: '解析失败', spin: false },
  };
  const status = statusConfig[novel.status];

  return (
    <motion.div
      layout
      initial={{ opacity: 0, scale: 0.95 }}
      animate={{ opacity: 1, scale: 1 }}
      exit={{ opacity: 0, scale: 0.95 }}
      whileHover={{ y: -4 }}
      transition={{ duration: 0.2 }}
      className="glass-card overflow-hidden cursor-pointer group"
      onClick={novel.status === 'ready' ? onOpen : undefined}
    >
      {/* 封面区域 */}
      <div
        className="relative h-40 flex items-center justify-center"
        style={{
          background: `linear-gradient(135deg, rgba(109,40,217,0.3), rgba(6,182,212,0.2))`,
        }}
      >
        <BookOpen size={40} style={{ color: 'rgba(255,255,255,0.3)' }} />

        {/* 状态徽章 */}
        <div
          className="absolute top-3 right-3 flex items-center gap-1.5 px-2 py-1 rounded-full text-xs"
          style={{
            background: 'rgba(0,0,0,0.6)',
            border: `1px solid ${status.color}40`,
            color: status.color,
          }}
        >
          <status.icon size={10} className={status.spin ? 'animate-spin' : ''} />
          {status.label}
        </div>

        {/* 删除按钮 */}
        <button
          onClick={(e) => { e.stopPropagation(); onDelete(); }}
          className="absolute top-3 left-3 p-1.5 rounded-lg opacity-0 group-hover:opacity-100 transition-opacity"
          style={{ background: 'rgba(239,68,68,0.2)', color: '#ef4444' }}
        >
          <Trash2 size={12} />
        </button>
      </div>

      {/* 信息区域 */}
      <div className="p-4">
        <h3 className="font-semibold text-sm mb-1 truncate" style={{ color: '#e2e8f0' }}>
          {novel.title}
        </h3>
        {novel.author && (
          <p className="text-xs mb-2 truncate" style={{ color: '#475569' }}>
            {novel.author}
          </p>
        )}
        <div className="flex items-center justify-between text-xs" style={{ color: '#334155' }}>
          <span>{novel.total_chapters > 0 ? `${novel.total_chapters} 章` : '—'}</span>
          <span className="flex items-center gap-1">
            <Clock size={10} />
            {new Date(novel.updated_at).toLocaleDateString('zh-CN')}
          </span>
        </div>

        {/* 类型标签 */}
        {novel.genre && (
          <div
            className="mt-2 inline-block px-2 py-0.5 rounded text-xs"
            style={{ background: 'rgba(109,40,217,0.15)', color: '#8b5cf6' }}
          >
            {novel.genre}
          </div>
        )}
      </div>
    </motion.div>
  );
}

function ImportModal({ onClose }: { onClose: () => void }) {
  const [title, setTitle] = useState('');
  const [author, setAuthor] = useState('');
  const [content, setContent] = useState('');
  const [deviationMode, setDeviationMode] = useState('canon');
  const importNovel = useImportNovel();

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!title.trim() || !content.trim()) return;
    await importNovel.mutateAsync({ title, author: author || undefined, content, deviation_mode: deviationMode });
    onClose();
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4"
      style={{ background: 'rgba(0,0,0,0.8)', backdropFilter: 'blur(8px)' }}
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <motion.div
        initial={{ opacity: 0, scale: 0.95, y: 20 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: 0.95, y: 20 }}
        className="glass-card w-full max-w-2xl max-h-[90vh] overflow-y-auto"
      >
        <div className="p-6">
          <h2 className="text-xl font-bold mb-6" style={{ fontFamily: 'var(--font-display)', color: '#e2e8f0' }}>
            导入小说
          </h2>

          <form onSubmit={handleSubmit} className="space-y-4">
            <div className="grid grid-cols-2 gap-4">
              <div>
                <label className="block text-xs font-semibold mb-1.5 uppercase tracking-wider" style={{ color: '#6d28d9' }}>
                  书名 *
                </label>
                <input
                  value={title}
                  onChange={(e) => setTitle(e.target.value)}
                  placeholder="输入小说名称"
                  required
                  className="w-full px-3 py-2 rounded-lg text-sm outline-none"
                  style={{
                    background: 'rgba(15,21,53,0.8)',
                    border: '1px solid rgba(109,40,217,0.3)',
                    color: '#e2e8f0',
                  }}
                />
              </div>
              <div>
                <label className="block text-xs font-semibold mb-1.5 uppercase tracking-wider" style={{ color: '#6d28d9' }}>
                  作者
                </label>
                <input
                  value={author}
                  onChange={(e) => setAuthor(e.target.value)}
                  placeholder="可选"
                  className="w-full px-3 py-2 rounded-lg text-sm outline-none"
                  style={{
                    background: 'rgba(15,21,53,0.8)',
                    border: '1px solid rgba(109,40,217,0.3)',
                    color: '#e2e8f0',
                  }}
                />
              </div>
            </div>

            <div>
              <label className="block text-xs font-semibold mb-1.5 uppercase tracking-wider" style={{ color: '#6d28d9' }}>
                故事偏离度
              </label>
              <div className="grid grid-cols-3 gap-2">
                {[
                  { value: 'canon', label: '忠实原著', desc: '严格遵循原著' },
                  { value: 'creative', label: '创意扩展', desc: '在原著基础上发挥' },
                  { value: 'remix', label: '自由改写', desc: '大胆改变走向' },
                ].map((opt) => (
                  <button
                    key={opt.value}
                    type="button"
                    onClick={() => setDeviationMode(opt.value)}
                    className="p-3 rounded-lg text-left transition-all"
                    style={{
                      background: deviationMode === opt.value ? 'rgba(109,40,217,0.2)' : 'rgba(15,21,53,0.6)',
                      border: `1px solid ${deviationMode === opt.value ? 'rgba(109,40,217,0.6)' : 'rgba(109,40,217,0.2)'}`,
                    }}
                  >
                    <div className="text-xs font-semibold" style={{ color: '#e2e8f0' }}>{opt.label}</div>
                    <div className="text-xs mt-0.5" style={{ color: '#475569' }}>{opt.desc}</div>
                  </button>
                ))}
              </div>
            </div>

            <div>
              <label className="block text-xs font-semibold mb-1.5 uppercase tracking-wider" style={{ color: '#6d28d9' }}>
                小说内容 *
              </label>
              <textarea
                value={content}
                onChange={(e) => setContent(e.target.value)}
                placeholder="粘贴小说全文内容（支持中英文，建议至少粘贴前3章用于角色提取）"
                rows={10}
                required
                className="w-full px-3 py-2 rounded-lg text-sm outline-none resize-none"
                style={{
                  background: 'rgba(15,21,53,0.8)',
                  border: '1px solid rgba(109,40,217,0.3)',
                  color: '#e2e8f0',
                  fontFamily: 'var(--font-reading)',
                  lineHeight: '1.8',
                }}
              />
              <p className="text-xs mt-1" style={{ color: '#334155' }}>
                字数：{content.length.toLocaleString()} 字
              </p>
            </div>

            <div className="flex justify-end gap-3 pt-2">
              <button
                type="button"
                onClick={onClose}
                className="px-5 py-2.5 rounded-lg text-sm transition-colors"
                style={{ color: '#94a3b8', background: 'rgba(255,255,255,0.05)' }}
              >
                取消
              </button>
              <button
                type="submit"
                disabled={importNovel.isPending || !title.trim() || !content.trim()}
                className="flex items-center gap-2 px-6 py-2.5 rounded-lg text-sm font-semibold transition-all"
                style={{
                  background: 'linear-gradient(135deg, #0891b2, #6d28d9)',
                  color: 'white',
                  opacity: importNovel.isPending ? 0.7 : 1,
                  cursor: importNovel.isPending ? 'not-allowed' : 'pointer',
                }}
              >
                {importNovel.isPending ? (
                  <><Loader2 size={14} className="animate-spin" /> 导入中...</>
                ) : (
                  <><Upload size={14} /> 开始导入</>
                )}
              </button>
            </div>
          </form>
        </div>
      </motion.div>
    </div>
  );
}

export function ShelfPage() {
  const navigate = useNavigate();
  const { data: novels, isLoading } = useNovels();
  const deleteNovel = useDeleteNovel();
  const [showImport, setShowImport] = useState(false);

  return (
    <div className="min-h-screen" style={{ background: 'var(--color-void)' }}>
      {/* 导航 */}
      <header
        className="sticky top-0 z-40 flex items-center justify-between px-6 py-4"
        style={{
          background: 'rgba(3,4,10,0.9)',
          backdropFilter: 'blur(20px)',
          borderBottom: '1px solid rgba(109,40,217,0.15)',
        }}
      >
        <div className="flex items-center gap-3">
          <div className="w-8 h-8 rounded-lg flex items-center justify-center"
            style={{ background: 'linear-gradient(135deg, #6d28d9, #06b6d4)' }}>
            <BookOpen size={16} color="white" />
          </div>
          <span className="font-bold" style={{ fontFamily: 'var(--font-display)', color: '#e2e8f0' }}>
            我的书架
          </span>
        </div>

        <button
          onClick={() => setShowImport(true)}
          className="flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-semibold transition-all"
          style={{
            background: 'linear-gradient(135deg, rgba(8,145,178,0.3), rgba(109,40,217,0.3))',
            border: '1px solid rgba(6,182,212,0.3)',
            color: '#22d3ee',
          }}
        >
          <Plus size={14} />
          导入小说
        </button>
      </header>

      <div className="p-6 max-w-6xl mx-auto">
        {isLoading ? (
          <div className="flex items-center justify-center h-64">
            <div className="w-8 h-8 border-2 rounded-full animate-spin" style={{ borderColor: '#6d28d9', borderTopColor: 'transparent' }} />
          </div>
        ) : novels?.length === 0 ? (
          <div className="text-center py-24">
            <BookOpen size={48} className="mx-auto mb-4 opacity-20" style={{ color: '#6d28d9' }} />
            <h3 className="text-lg font-semibold mb-2" style={{ color: '#e2e8f0' }}>书架还是空的</h3>
            <p className="text-sm mb-6" style={{ color: '#475569' }}>导入你的第一本小说，开始沉浸式体验</p>
            <button
              onClick={() => setShowImport(true)}
              className="btn-cosmic-filled px-6 py-3 rounded-xl text-sm font-semibold"
              style={{ background: 'linear-gradient(135deg, #0891b2, #6d28d9)', color: 'white', border: 'none', cursor: 'pointer' }}
            >
              导入小说
            </button>
          </div>
        ) : (
          <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 gap-4">
            <AnimatePresence>
              {novels?.map((novel) => (
                <NovelCard
                  key={novel.id}
                  novel={novel}
                  onOpen={() => navigate(`/reader/${novel.id}/1`)}
                  onDelete={() => deleteNovel.mutate(novel.id)}
                />
              ))}
            </AnimatePresence>
          </div>
        )}
      </div>

      <AnimatePresence>
        {showImport && <ImportModal onClose={() => setShowImport(false)} />}
      </AnimatePresence>
    </div>
  );
}
