import { translate as t, useLocale } from '@/shared/lib/i18n';
import { useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import * as Dialog from '@radix-ui/react-dialog';
import { motion, AnimatePresence } from 'framer-motion';
import { Plus, BookOpen, Clock, BookMinus, Loader2, CheckCircle, AlertCircle, RotateCcw, Settings, Library, Sparkles } from 'lucide-react';
import {
  useNovels,
  useDeleteNovel,
  useRetryNovel,
  useNovelCatalog,
  useAttachNovel,
} from '@/entities/novel';
import { useAuthStore } from '@/features/auth';
import { NovelImportModal } from '@/features/novel-import';
import { WorldSeriesDialog } from '@/features/novel-world-series';
import type { Novel } from '@/shared/types';
import { getApiErrorMessage } from '@/shared/api/client';
import { toast } from 'sonner';

const importFailureGuidance: Record<string, { message: string; action: 'retry' | 'import' }> = {
  'The retained source file is missing; re-upload the source': { get message() { return t("The original file is unavailable. Import the novel again."); }, action: 'import' },
  'No parsed chapters are available; re-upload the source': { get message() { return t("No chapter content is available. Import the original file again."); }, action: 'import' },
  'The retained source file cannot be parsed; re-upload the source': { get message() { return t("The original file could not be parsed. Check the file and import it again."); }, action: 'import' },
  'Import provider budget exhausted; re-upload the source': { get message() { return t("Retry limit reached. Import the original file again."); }, action: 'import' },
  'Import exceeded the processing budget; re-upload a shorter source': { get message() { return t("This parsing run reached its processing limit. Import a shorter file."); }, action: 'import' },
  'AI response reached its output limit; import a shorter source': { get message() { return t("The model response reached its length limit. Shorten or split the source text and import it again."); }, action: 'import' },
  'AI provider balance is unavailable; contact the site administrator before retrying': { get message() { return t("The model service has insufficient funds. Ask the site administrator to resolve it, then retry."); }, action: 'retry' },
  'AI provider balance is unavailable; contact the site administrator, then re-upload the source': { get message() { return t("The model service has insufficient funds and the retry limit was reached. Ask the site administrator to resolve it, then import again."); }, action: 'import' },
  'AI provider rejected the request; contact the site administrator before retrying': { get message() { return t("The model service rejected the request. Ask the site administrator to resolve it, then retry."); }, action: 'retry' },
  'AI provider rejected the request; contact the site administrator, then re-upload the source': { get message() { return t("The model service rejected the request and the retry limit was reached. Ask the site administrator to resolve it, then import again."); }, action: 'import' },
  'Source storage is unavailable; retry the import': { get message() { return t("File storage is unavailable. Retry parsing later."); }, action: 'retry' },
  'Import processing failed; retry the import': { get message() { return t("No specific reason was recorded. You can retry parsing."); }, action: 'retry' },
  'Previous import failed; retry or re-upload the source': { get message() { return t("No specific reason was recorded. Retry parsing; if it fails again, import the original file again."); }, action: 'retry' },
  'Chapter boundary analysis did not finish; retry the import': { get message() { return t("Chapter boundary analysis is incomplete. You can retry parsing."); }, action: 'retry' },
  'Character analysis did not finish; retry the import': { get message() { return t("Character analysis is incomplete. You can retry parsing."); }, action: 'retry' },
  'Story model analysis did not finish; retry the import': { get message() { return t("Story model analysis is incomplete. You can retry parsing."); }, action: 'retry' },
  'AI request for story model analysis failed; retry the import': { get message() { return t("Story model analysis request failed. You can retry parsing."); }, action: 'retry' },
  'AI story model response could not be validated; retry the import': { get message() { return t("Story model analysis could not be validated. You can retry parsing."); }, action: 'retry' },
  'AI story model response could not be validated; re-upload the source': { get message() { return t("Story model analysis could not be validated and the retry limit was reached. Import the original file again."); }, action: 'import' },
  'Story model checkpoint could not be saved; retry the import': { get message() { return t("Story model analysis progress could not be saved. You can retry parsing."); }, action: 'retry' },
};

function getFailureGuidance(parseError?: string) {
  return parseError && Object.prototype.hasOwnProperty.call(importFailureGuidance, parseError)
    ? importFailureGuidance[parseError]
    : undefined;
}

function getRetryErrorMessage(error: unknown) {
  const message = getApiErrorMessage(error, '');
  const guidance = getFailureGuidance(message);
  if (guidance) return guidance.message;
  switch (message) {
    case 'Novel import capacity is busy; retry the request':
      return t("Parsing is busy. Try again later.");
    case 'Novel exceeds the supported processing budget':
      return t("Content exceeds the processing limit. Import a shorter file.");
    case 'Only failed imports can be retried':
      return t("This novel does not currently need parsing retried.");
    case 'Novel cannot be retried':
    case 'Import cannot be retried':
      return t("Cannot retry further. Refresh the shelf; if it still fails, import the original file again.");
    default:
      return t("Retry failed. Try again later.");
  }
}

function NovelCard({ novel, onOpen, onDelete, onRetry, onImport, onManageWorldSeries, retrying }: {
  novel: Novel;
  onOpen: () => void;
  onDelete: () => void;
  onRetry: () => void;
  onImport: () => void;
  onManageWorldSeries: () => void;
  retrying: boolean;
}) {
  const locale = useLocale();
  const failureGuidance = getFailureGuidance(novel.parse_error);
  const statusConfig = {
    pending: { icon: Loader2, color: '#5f6368', label: t("Awaiting parsing"), spin: true },
    parsing: { icon: Loader2, color: '#0b57d0', label: t("Parsing…"), spin: true },
    ready: { icon: CheckCircle, color: '#188038', label: t("Ready"), spin: false },
    error: { icon: AlertCircle, color: '#b3261e', label: t("Parsing failed"), spin: false },
  };
  const status = statusConfig[novel.status] ?? {
    icon: AlertCircle,
    color: '#f59e0b',
    label: t("Unknown status"),
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
      {/* Cover */}
      <div
        className="relative flex h-44 items-center justify-center bg-[#eef3ff]"
      >
        <BookOpen size={42} style={{ color: '#7b8db7' }} />

        {/* Status badge */}
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

        {/* Remove button */}
        <button
          onClick={(e) => { e.stopPropagation(); onDelete(); }}
          aria-label={t("Remove {p0} from the shelf", { p0: novel.title })}
          className="absolute top-3 left-3 rounded-lg p-1.5 opacity-80 transition-opacity hover:opacity-100 focus-visible:opacity-100"
          title={t("Remove from shelf (personal world retained)")}
          style={{ background: '#f1f3f4', color: '#5f6368' }}
        >
          <BookMinus size={12} />
        </button>
      </div>

      {/* Novel details */}
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
          <span>{novel.total_chapters > 0 ? t("{p0} chapters", { p0: novel.total_chapters }) : '—'}</span>
          <span className="flex items-center gap-1">
            <Clock size={10} />
            {new Date(novel.updated_at).toLocaleDateString(locale)}
          </span>
        </div>

        {/* Genre label */}
        {novel.genre && (
          <div
            className="mt-2 inline-block px-2 py-0.5 rounded text-xs"
            style={{ background: '#e8f0fe', color: '#174ea6' }}
          >
            {novel.genre}
          </div>
        )}
        {novel.status === 'ready' ? (
          <button
            type="button"
            className="mt-3 flex w-full items-center justify-center gap-2 rounded-lg border border-[#d2e3fc] px-3 py-2 text-xs font-medium text-[#174ea6] hover:bg-[#f3f7ff]"
            onClick={event => { event.stopPropagation(); onManageWorldSeries(); }}
          >
            <Sparkles size={13} />
            {t("Series management")}
          </button>
        ) : null}
        {novel.status === 'error' && (
          <>
            <p
              className="mt-3 text-xs leading-relaxed"
              role="alert"
              style={{ color: '#b3261e' }}
            >
              {t("Parsing failed:")}{failureGuidance?.message ?? t("No specific reason was recorded. Try parsing again; if retries are exhausted, import the original file again.")}
            </p>
            <button
              type="button"
              disabled={retrying}
              onClick={(event) => {
                event.stopPropagation();
                if (failureGuidance?.action === 'import') onImport();
                else onRetry();
              }}
              className="mt-3 flex w-full items-center justify-center gap-1.5 rounded-lg px-3 py-2 text-xs font-semibold"
              style={{
                background: '#e8f0fe',
                border: '1px solid #a8c7fa',
                color: '#0b57d0',
                opacity: retrying ? 0.6 : 1,
              }}
            >
              {retrying ? <Loader2 size={12} className="animate-spin" /> : failureGuidance?.action === 'import' ? <Plus size={12} /> : <RotateCcw size={12} />}
              {retrying ? t("Retrying…") : failureGuidance?.action === 'import' ? t("Import the file again") : t("Retry parsing")}
            </button>
          </>
        )}
      </div>
    </motion.div>
  );
}

function SharedLibraryModal({ onClose, shelfNovels }: { onClose: () => void; shelfNovels?: Novel[] }) {
  useLocale();
  const returnFocusRef = useRef<HTMLElement | null>(
    typeof document !== 'undefined' && document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null,
  );
  const { data: novels, isLoading, isError, refetch } = useNovelCatalog();
  const attachNovel = useAttachNovel();
  const [deviationMode, setDeviationMode] = useState('canon');
  const shelfNovelIds = new Set(shelfNovels?.map(novel => novel.id));

  const attach = async (novelId: string) => {
    try {
      await attachNovel.mutateAsync({ novelId, deviationMode });
      toast.success(t("Added to my shelf"));
    } catch (error) {
      toast.error(getApiErrorMessage(error, t("Could not add to shelf")));
    }
  };

  return (
    <Dialog.Root open onOpenChange={(open) => { if (!open) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay
          className="fixed inset-0 z-50"
          style={{ background: 'rgba(32,33,36,0.42)', backdropFilter: 'blur(8px)' }}
        />
        <Dialog.Content
          className="surface-card fixed left-1/2 top-1/2 z-50 flex max-h-[80vh] w-[calc(100%_-_2rem)] max-w-2xl -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden outline-none"
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            const returnFocus = returnFocusRef.current;
            if (returnFocus?.isConnected) returnFocus.focus();
          }}
        >
        <div className="border-b border-[#e8eaed] px-6 py-5 sm:px-8">
          <Dialog.Title className="text-2xl font-medium text-[#1f1f1f]">{t("Shared library")}</Dialog.Title>
          <Dialog.Description className="mt-2 text-sm text-[#5f6368]">{t("Add parsed novels directly. Your progress, identity and world are saved independently.")}</Dialog.Description>
          <div className="mt-4 flex flex-wrap gap-2" role="group" aria-label={t("Story deviation")}>
            {[
              { value: 'canon', label: t("Follow the original") },
              { value: 'creative', label: t("Creative expansion") },
              { value: 'remix', label: t("Free rewriting") },
            ].map(option => (
              <button
                key={option.value}
                type="button"
                aria-pressed={deviationMode === option.value}
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
            <div className="flex h-32 items-center justify-center" role="status" aria-label={t("Loading shared library")}><Loader2 className="animate-spin text-[#0b57d0]" /></div>
          ) : isError && !novels ? (
            <div className="py-12 text-center text-sm text-[#5f6368]" role="alert">
              <p>{t("Shared library failed to load.")}</p>
              <button type="button" onClick={() => refetch()} className="tonal-action mt-3 text-xs">{t("Retry")}</button>
            </div>
          ) : novels?.length ? novels.map(novel => {
            const attaching = attachNovel.isPending && attachNovel.variables?.novelId === novel.id;
            const onShelf = shelfNovelIds.has(novel.id);
            const shelfUnavailable = !shelfNovels;
            return (
              <div key={novel.id} className="flex items-center gap-4 rounded-xl border border-[#e1e3e8] p-4">
                <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg bg-[#eef3ff] text-[#174ea6]"><BookOpen size={19} /></div>
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-semibold text-[#1f1f1f]">{novel.title}</p>
                  <p className="mt-1 truncate text-xs text-[#5f6368]">{novel.author || t("Unknown author")} · {t("{p0} chapters", { p0: novel.total_chapters })}</p>
                </div>
                <button
                  type="button"
                  aria-label={onShelf ? t("“{p0}” is already on your shelf", { p0: novel.title }) : shelfUnavailable ? t("Shelf unavailable. Cannot add “{p0}”", { p0: novel.title }) : t("Add “{p0}” to your shelf", { p0: novel.title })}
                  disabled={onShelf || shelfUnavailable || attachNovel.isPending}
                  onClick={() => attach(novel.id)}
                  className="primary-action shrink-0 text-xs"
                >
                  {onShelf ? <CheckCircle size={13} /> : attaching ? <Loader2 size={13} className="animate-spin" /> : <Plus size={13} />}
                  {onShelf ? t("Already on shelf") : shelfUnavailable ? t("Shelf unavailable") : t("Add to shelf")}
                </button>
              </div>
            );
          }) : (
            <div className="py-12 text-center text-sm text-[#5f6368]">{t("No parsed novels yet. Upload a new novel.")}</div>
          )}
        </div>
        <div className="flex justify-end border-t border-[#e8eaed] px-6 py-4 sm:px-8">
          <Dialog.Close asChild>
            <button type="button" className="tonal-action text-sm">{t("Done")}</button>
          </Dialog.Close>
        </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

export function ShelfPage() {
  useLocale();
  const navigate = useNavigate();
  const user = useAuthStore(state => state.user);
  const {
    data: novels,
    isLoading,
    isError,
    refetch,
  } = useNovels();
  const deleteNovel = useDeleteNovel(user?.id);
  const retryNovel = useRetryNovel();
  const processingCount = novels?.filter(
    novel => novel.status === 'pending' || novel.status === 'parsing',
  ).length ?? 0;
  const [showImport, setShowImport] = useState(false);
  const [showSharedLibrary, setShowSharedLibrary] = useState(false);
  const [worldSeriesNovel, setWorldSeriesNovel] = useState<Novel>();

  return (
    <div className="app-surface min-h-screen">
      {/* Navigation */}
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
          <button type="button" aria-label={t("Settings")} onClick={() => navigate('/settings')} className="flex h-10 w-10 items-center justify-center rounded-full text-[#0b57d0] transition-colors hover:bg-[#e8f0fe]">
            <Settings size={16} />
          </button>
          <button
            type="button"
            aria-label={t("Open shared library")}
            onClick={() => setShowSharedLibrary(true)}
            className="tonal-action px-3 text-sm sm:px-4"
          >
            <Library size={14} />
            <span className="hidden sm:inline">{t("Shared library")}</span>
          </button>
          <button
            type="button"
            aria-label={t("Import novels")}
            onClick={() => setShowImport(true)}
            className="primary-action px-3 text-sm sm:px-5"
          >
            <Plus size={14} />
            <span className="hidden sm:inline">{t("Import novels")}</span>
          </button>
        </div>
      </header>

      <main className="mx-auto max-w-6xl px-4 py-8 sm:px-6 sm:py-10">
        <div className="mb-7">
          <p className="text-sm font-medium text-[#0b57d0]">{t("Personal library")}</p>
          <h1 className="mt-2 text-3xl font-medium tracking-[-0.02em] text-[#1f1f1f]">{t("My shelf")}</h1>
          <p className="mt-2 text-sm text-[#5f6368]">{t("Manage imported novels and continue exploring from where you left off.")}</p>
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
            {t("Parsing")} {processingCount} {t("novels. Status updates automatically.")}
          </div>
        )}
        {isLoading ? (
          <div className="flex items-center justify-center h-64">
            <div className="w-8 h-8 border-2 rounded-full animate-spin" style={{ borderColor: '#0b57d0', borderTopColor: 'transparent' }} />
          </div>
        ) : isError && !novels ? (
          <div className="surface-card px-6 py-16 text-center" role="alert">
            <span className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-[#fce8e6] text-[#b3261e]">
              <AlertCircle size={24} aria-hidden="true" />
            </span>
            <h2 className="mt-5 text-xl font-semibold text-[#1f1f1f]">{t("Cannot load the shelf right now")}</h2>
            <p className="mx-auto mt-2 max-w-md text-sm leading-6 text-[#5f6368]">{t("Shelf failed to load. Your imported novels remain safe.")}</p>
            <button className="primary-action mt-6" onClick={() => refetch()}>{t("Retry")}</button>
          </div>
        ) : novels?.length === 0 ? (
          <div className="surface-card py-20 text-center">
            <BookOpen size={48} className="mx-auto mb-4" style={{ color: '#7b8db7' }} />
            <h3 className="text-lg font-semibold mb-2" style={{ color: '#1f1f1f' }}>{t("Your shelf is empty")}</h3>
            <p className="text-sm mb-6" style={{ color: '#5f6368' }}>{t("Import your first novel and start exploring")}</p>
            <button
              onClick={() => setShowImport(true)}
              className="primary-action text-sm"
            >
              {t("Import novels")}
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
                    onSuccess: () => toast.success(t("Removed from shelf. Add it again to continue your original world.")),
                    onError: (error) => toast.error(getApiErrorMessage(error, t("Could not remove from shelf"))),
                  })}
                  onRetry={() => retryNovel.mutate(novel.id, {
                    onSuccess: () => toast.success(t("Parsing restarted")),
                    onError: (error) => toast.error(getRetryErrorMessage(error)),
                  })}
                  onImport={() => setShowImport(true)}
                  onManageWorldSeries={() => setWorldSeriesNovel(novel)}
                  retrying={retryNovel.isPending && retryNovel.variables === novel.id}
                />
              ))}
            </AnimatePresence>
          </div>
        )}
      </main>

      <AnimatePresence>
        {showImport && <NovelImportModal onClose={() => setShowImport(false)} />}
        {showSharedLibrary && <SharedLibraryModal onClose={() => setShowSharedLibrary(false)} shelfNovels={novels} />}
        {worldSeriesNovel && user?.id ? (
          <WorldSeriesDialog
            key={`${user.id}:${worldSeriesNovel.id}`}
            principalId={user.id}
            isPrincipalCurrent={() => useAuthStore.getState().user?.id === user.id}
            novel={worldSeriesNovel}
            readyNovels={novels?.filter(novel => novel.status === 'ready') ?? []}
            onClose={() => setWorldSeriesNovel(undefined)}
          />
        ) : null}
      </AnimatePresence>
    </div>
  );
}
