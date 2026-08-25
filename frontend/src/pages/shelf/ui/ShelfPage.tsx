import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { motion, AnimatePresence } from 'framer-motion';
import { Plus, BookOpen, Clock, BookMinus, Loader2, CheckCircle, AlertCircle, RotateCcw, Settings, Library } from 'lucide-react';
import {
  useNovels,
  useDeleteNovel,
  useRetryNovel,
  useNovelCatalog,
  useAttachNovel,
} from '@/entities/novel';
import { useAuthStore } from '@/features/auth';
import { NovelImportModal } from '@/features/novel-import';
import type { Novel } from '@/shared/types';
import { getApiErrorMessage } from '@/shared/api/client';
import { toast } from 'sonner';

function NovelCard({ novel, onOpen, onDelete, onRetry, retrying }: {
  novel: Novel;
  onOpen: () => void;
  onDelete: () => void;
  onRetry: () => void;
  retrying: boolean;
}) {
  const statusConfig = {
    pending: { icon: Loader2, color: '#5f6368', label: '等待解析', spin: true },
    parsing: { icon: Loader2, color: '#0b57d0', label: '解析中…', spin: true },
    ready: { icon: CheckCircle, color: '#188038', label: '已就绪', spin: false },
    error: { icon: AlertCircle, color: '#b3261e', label: '解析失败', spin: false },
  };
  const status = statusConfig[novel.status] ?? {
    icon: AlertCircle,
    color: '#f59e0b',
    label: '状态未知',
    spin: false,
  };

  return (
    <motion.div
      layout
      initial={{ opacity: 0, scale: 0.95 }}
      animate={{ opacity: 1, scale: 1 }}
      exit={{ opacity: 0, scale: 0.95 }}
      whileHover={{ y: -4 }}
      transition={{ duration: 0.2 }}
      className="surface-card group cursor-pointer overflow-hidden"
      onClick={novel.status === 'ready' ? onOpen : undefined}
    >
      {/* 封面区域 */}
      <div
        className="relative flex h-44 items-center justify-center bg-[#eef3ff]"
      >
        <BookOpen size={42} style={{ color: '#7b8db7' }} />

        {/* 状态徽章 */}
        <div
          className="absolute top-3 right-3 flex items-center gap-1.5 px-2 py-1 rounded-full text-xs"
          style={{
            background: 'rgba(255,255,255,0.92)',
            border: `1px solid ${status.color}35`,
            color: status.color,
          }}
        >
          <status.icon size={10} className={status.spin ? 'animate-spin' : ''} />
          {status.label}
        </div>

        {/* 删除按钮 */}
        <button
          onClick={(e) => { e.stopPropagation(); onDelete(); }}
          aria-label={`将 ${novel.title} 移出书架`}
          className={`absolute top-3 left-3 p-1.5 rounded-lg transition-opacity ${
            novel.status === 'error' ? 'opacity-100' : 'opacity-0 group-hover:opacity-100'
          }`}
          title="移出书架（个人世界会保留）"
          style={{ background: '#f1f3f4', color: '#5f6368' }}
        >
          <BookMinus size={12} />
        </button>
      </div>

      {/* 信息区域 */}
      <div className="p-4">
        {novel.status === 'ready' ? (
          <button
            type="button"
            onClick={(e) => { e.stopPropagation(); onOpen(); }}
            className="font-semibold text-sm mb-1 truncate text-left max-w-full"
            style={{ color: '#1f1f1f' }}
          >
            {novel.title}
          </button>
        ) : (
          <h3 className="font-semibold text-sm mb-1 truncate" style={{ color: '#1f1f1f' }}>
            {novel.title}
          </h3>
        )}
        {novel.author && (
          <p className="text-xs mb-2 truncate" style={{ color: '#5f6368' }}>
            {novel.author}
          </p>
        )}
        <div className="flex items-center justify-between text-xs" style={{ color: '#5f6368' }}>
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
            style={{ background: '#e8f0fe', color: '#174ea6' }}
          >
            {novel.genre}
          </div>
        )}
        {novel.status === 'error' && novel.parse_error && (
          <>
            <p
              className="mt-3 text-xs leading-relaxed"
              role="alert"
              style={{ color: '#b3261e' }}
            >
              解析失败：{novel.parse_error.includes('EOF while parsing') || novel.parse_error.includes('empty response')
                ? 'AI 服务返回了空内容，可以直接重试'
                : novel.parse_error}
            </p>
            <button
              type="button"
              disabled={retrying}
              onClick={(event) => {
                event.stopPropagation();
                onRetry();
              }}
              className="mt-3 flex w-full items-center justify-center gap-1.5 rounded-lg px-3 py-2 text-xs font-semibold"
              style={{
                background: '#e8f0fe',
                border: '1px solid #a8c7fa',
                color: '#0b57d0',
                opacity: retrying ? 0.6 : 1,
              }}
            >
              {retrying ? <Loader2 size={12} className="animate-spin" /> : <RotateCcw size={12} />}
              {retrying ? '正在重试...' : '重试解析'}
            </button>
          </>
        )}
      </div>
    </motion.div>
  );
}

function SharedLibraryModal({ onClose }: { onClose: () => void }) {
  const { data: novels, isLoading, isError, refetch } = useNovelCatalog();
  const attachNovel = useAttachNovel();
  const [deviationMode, setDeviationMode] = useState('canon');

  const attach = async (novelId: string) => {
    try {
      await attachNovel.mutateAsync({ novelId, deviationMode });
      toast.success('已加入我的书架');
    } catch (error) {
      toast.error(getApiErrorMessage(error, '加入书架失败'));
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4"
      style={{ background: 'rgba(32,33,36,0.42)', backdropFilter: 'blur(8px)' }}
      onClick={(event) => event.target === event.currentTarget && onClose()}
    >
      <motion.div
        initial={{ opacity: 0, scale: 0.95, y: 20 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: 0.95, y: 20 }}
        className="surface-card flex max-h-[80vh] w-full max-w-2xl flex-col overflow-hidden"
      >
        <div className="border-b border-[#e8eaed] px-6 py-5 sm:px-8">
          <h2 className="text-2xl font-medium text-[#1f1f1f]">共享书库</h2>
          <p className="mt-2 text-sm text-[#5f6368]">直接加入已解析的小说；你的进度、身份和世界线独立保存。</p>
          <div className="mt-4 flex flex-wrap gap-2">
            {[
              { value: 'canon', label: '忠实原著' },
              { value: 'creative', label: '创意扩展' },
              { value: 'remix', label: '自由改写' },
            ].map(option => (
              <button
                key={option.value}
                type="button"
                onClick={() => setDeviationMode(option.value)}
                className="rounded-full px-3 py-1.5 text-xs font-medium"
                style={{
                  background: deviationMode === option.value ? '#e8f0fe' : '#f8fafd',
                  color: deviationMode === option.value ? '#0b57d0' : '#5f6368',
                  border: `1px solid ${deviationMode === option.value ? '#a8c7fa' : '#dadce0'}`,
                }}
              >
                {option.label}
              </button>
            ))}
          </div>
        </div>
        <div className="min-h-40 space-y-3 overflow-y-auto px-6 py-5 sm:px-8">
          {isLoading ? (
            <div className="flex h-32 items-center justify-center"><Loader2 className="animate-spin text-[#0b57d0]" /></div>
          ) : isError ? (
            <div className="py-12 text-center text-sm text-[#5f6368]">
              <p>共享书库加载失败。</p>
              <button type="button" onClick={() => refetch()} className="tonal-action mt-3 text-xs">重试</button>
            </div>
          ) : novels?.length ? novels.map(novel => {
            const attaching = attachNovel.isPending && attachNovel.variables?.novelId === novel.id;
            return (
              <div key={novel.id} className="flex items-center gap-4 rounded-xl border border-[#e1e3e8] p-4">
                <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg bg-[#eef3ff] text-[#174ea6]"><BookOpen size={19} /></div>
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-semibold text-[#1f1f1f]">{novel.title}</p>
                  <p className="mt-1 truncate text-xs text-[#5f6368]">{novel.author || '作者未知'} · {novel.total_chapters} 章</p>
                </div>
                <button type="button" disabled={attachNovel.isPending} onClick={() => attach(novel.id)} className="primary-action shrink-0 text-xs">
                  {attaching ? <Loader2 size={13} className="animate-spin" /> : <Plus size={13} />}
                  加入书架
                </button>
              </div>
            );
          }) : (
            <div className="py-12 text-center text-sm text-[#5f6368]">暂无可加入的小说，可以上传一本新小说。</div>
          )}
        </div>
        <div className="flex justify-end border-t border-[#e8eaed] px-6 py-4 sm:px-8">
          <button type="button" onClick={onClose} className="tonal-action text-sm">完成</button>
        </div>
      </motion.div>
    </div>
  );
}

export function ShelfPage() {
  const navigate = useNavigate();
  const user = useAuthStore(state => state.user);
  const { data: novels, isLoading } = useNovels();
  const deleteNovel = useDeleteNovel(user?.id);
  const retryNovel = useRetryNovel();
  const processingCount = novels?.filter(
    novel => novel.status === 'pending' || novel.status === 'parsing',
  ).length ?? 0;
  const [showImport, setShowImport] = useState(false);
  const [showSharedLibrary, setShowSharedLibrary] = useState(false);

  return (
    <div className="app-surface min-h-screen">
      {/* 导航 */}
      <header
        className="sticky top-0 z-40 flex items-center justify-between border-b border-[#e1e3e8] bg-white/95 px-4 py-3 backdrop-blur-xl sm:px-6"
        style={{
          backdropFilter: 'blur(20px)',
        }}
      >
        <div className="flex items-center gap-3">
          <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-[#0b57d0]">
            <BookOpen size={16} color="white" />
          </div>
          <span className="hidden font-semibold text-[#174ea6] sm:inline">
            NovelWorld
          </span>
        </div>

        <div className="flex items-center gap-2">
          <button type="button" aria-label="设置" onClick={() => navigate('/settings')} className="flex h-10 w-10 items-center justify-center rounded-full text-[#0b57d0] transition-colors hover:bg-[#e8f0fe]">
            <Settings size={16} />
          </button>
          <button
            type="button"
            aria-label="打开共享书库"
            onClick={() => setShowSharedLibrary(true)}
            className="tonal-action px-3 text-sm sm:px-4"
          >
            <Library size={14} />
            <span className="hidden sm:inline">共享书库</span>
          </button>
          <button
            type="button"
            aria-label="导入小说"
            onClick={() => setShowImport(true)}
            className="primary-action px-3 text-sm sm:px-5"
          >
            <Plus size={14} />
            <span className="hidden sm:inline">导入小说</span>
          </button>
        </div>
      </header>

      <main className="mx-auto max-w-6xl px-4 py-8 sm:px-6 sm:py-10">
        <div className="mb-7">
          <p className="text-sm font-medium text-[#0b57d0]">个人书库</p>
          <h1 className="mt-2 text-3xl font-medium tracking-[-0.02em] text-[#1f1f1f]">我的书架</h1>
          <p className="mt-2 text-sm text-[#5f6368]">管理已导入的小说，并从上次的位置继续探索。</p>
        </div>
        {processingCount > 0 && (
          <div
            role="status"
            className="mb-5 flex items-center gap-3 rounded-xl px-4 py-3 text-sm"
            style={{
              background: '#e8f0fe',
              border: '1px solid #a8c7fa',
              color: '#174ea6',
            }}
          >
            <Loader2 size={16} className="animate-spin" />
            正在解析 {processingCount} 本小说，状态会自动更新
          </div>
        )}
        {isLoading ? (
          <div className="flex items-center justify-center h-64">
            <div className="w-8 h-8 border-2 rounded-full animate-spin" style={{ borderColor: '#0b57d0', borderTopColor: 'transparent' }} />
          </div>
        ) : novels?.length === 0 ? (
          <div className="surface-card py-20 text-center">
            <BookOpen size={48} className="mx-auto mb-4" style={{ color: '#7b8db7' }} />
            <h3 className="text-lg font-semibold mb-2" style={{ color: '#1f1f1f' }}>书架还是空的</h3>
            <p className="text-sm mb-6" style={{ color: '#5f6368' }}>导入你的第一本小说，开始沉浸式体验</p>
            <button
              onClick={() => setShowImport(true)}
              className="primary-action text-sm"
            >
              导入小说
            </button>
          </div>
        ) : (
          <div className="grid grid-cols-1 gap-5 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-4">
            <AnimatePresence>
              {novels?.map((novel) => (
                <NovelCard
                  key={novel.id}
                  novel={novel}
                  onOpen={() => navigate(`/reader/${novel.id}`)}
                  onDelete={() => deleteNovel.mutate(novel.id, {
                    onSuccess: () => toast.success('已移出书架，重新加入后可继续原来的世界'),
                    onError: (error) => toast.error(getApiErrorMessage(error, '移出书架失败')),
                  })}
                  onRetry={() => retryNovel.mutate(novel.id, {
                    onSuccess: () => toast.success('已重新开始解析'),
                    onError: (error) => toast.error(getApiErrorMessage(error, '重试失败')),
                  })}
                  retrying={retryNovel.isPending && retryNovel.variables === novel.id}
                />
              ))}
            </AnimatePresence>
          </div>
        )}
      </main>

      <AnimatePresence>
        {showImport && <NovelImportModal onClose={() => setShowImport(false)} />}
        {showSharedLibrary && <SharedLibraryModal onClose={() => setShowSharedLibrary(false)} />}
      </AnimatePresence>
    </div>
  );
}
