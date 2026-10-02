import { translate as t, useLocale } from '@/shared/lib/i18n';
import { useRef, useState, type FormEvent } from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import { Loader2, Upload, X } from 'lucide-react';
import { toast } from 'sonner';
import {
  novelTitleFromFile,
  NovelBatchUploadError,
  useImportNovel,
  useUploadNovel,
  useUploadNovelsBatch,
  validateNovelBatchFiles,
} from '@/entities/novel';
import { getApiErrorCode, getApiErrorMessage } from '@/shared/api/client';

export function NovelImportModal({ onClose }: { onClose: () => void }) {
  const locale = useLocale();
  const returnFocusRef = useRef<HTMLElement | null>(
    typeof document !== 'undefined' && document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null,
  );
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [title, setTitle] = useState('');
  const [author, setAuthor] = useState('');
  const [content, setContent] = useState('');
  const [files, setFiles] = useState<File[]>([]);
  const [submitting, setSubmitting] = useState(false);
  const [unknownFiles, setUnknownFiles] = useState<string[]>([]);
  const [deviationMode, setDeviationMode] = useState('canon');
  const importNovel = useImportNovel();
  const uploadNovel = useUploadNovel();
  const uploadBatch = useUploadNovelsBatch();
  const isPending = submitting || importNovel.isPending || uploadNovel.isPending || uploadBatch.isPending;
  const isBatch = files.length > 1;

  const selectFiles = (selected: File[]) => {
    if (!selected.length) return;
    const error = validateNovelBatchFiles(selected);
    if (error) {
      toast.error(error);
      return;
    }
    setFiles(selected);
    setContent('');
    setTitle(selected.length === 1 ? novelTitleFromFile(selected[0]) : '');
  };

  const removeFile = (index: number) => {
    const next = files.filter((_, fileIndex) => fileIndex !== index);
    setFiles(next);
    setTitle(next.length === 1 ? novelTitleFromFile(next[0]) : '');
  };

  const handleSubmit = async (event: FormEvent) => {
    event.preventDefault();
    if ((!isBatch && !title.trim()) || (!files.length && !content.trim())) return;
    if (isPending) return;
    setSubmitting(true);
    try {
      if (isBatch) {
        await uploadBatch.mutateAsync({
          author: author || undefined,
          deviationMode,
          files,
        });
      } else if (files.length === 1) {
        await uploadNovel.mutateAsync({
          title,
          author: author || undefined,
          deviationMode,
          file: files[0],
        });
      } else {
        await importNovel.mutateAsync({
          title,
          author: author || undefined,
          content,
          deviation_mode: deviationMode,
        });
      }
      toast.success(isBatch ? t("{p0} novels accepted, awaiting parsing", { p0: files.length }) : t("Novel accepted, awaiting parsing"));
      onClose();
    } catch (error) {
      if (error instanceof NovelBatchUploadError) {
        if (error.reason === 'session_changed') {
          toast.error(t("Your session changed. Remaining uploads stopped. Check the original account's shelf."));
          onClose();
          return;
        }
        setFiles(error.remainingFiles);
        setTitle(error.remainingFiles.length === 1 ? novelTitleFromFile(error.remainingFiles[0]) : '');
        if (error.unknownFiles.length) {
          setUnknownFiles(error.unknownFiles.map(file => file.name));
          toast.error(t("{p0} novels confirmed accepted; {p1} outcomes are unknown. Check your shelf before uploading again.", { p0: error.accepted.length, p1: error.unknownFiles.length }));
          return;
        }
        const prefix = error.accepted.length ? t("{p0} novels accepted; remaining files were not accepted.", { p0: error.accepted.length }) : '';
        toast.error(prefix + (getApiErrorCode(error.cause) === 'upload_capacity_busy'
          ? t("Uploads are busy. Try again later.") : getApiErrorMessage(error.cause, t("Upload failed. Check the remaining files and try again."))));
        return;
      }
      const code = getApiErrorCode(error);
      const message = code === 'upload_capacity_busy'
        ? t("Uploads are busy. Try again later.")
        : code === 'import_capacity_busy'
        ? t("Parsing is busy. Import again later.")
        : code === 'source_storage_unavailable'
          ? t("File storage is temporarily unavailable. Try again later.")
          : code === 'service_unavailable' || code === 'bad_gateway'
            ? t("The import service is temporarily unavailable. Try again later.")
            : getApiErrorMessage(error, isBatch ? t("Batch import failed") : t("Novel import failed"));
      toast.error(message);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Dialog.Root open onOpenChange={(open) => { if (!open && !isPending) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay
          className="fixed inset-0 z-50"
          style={{ background: 'rgba(32,33,36,0.42)', backdropFilter: 'blur(8px)' }}
        />
        <Dialog.Content
          className="surface-card fixed left-1/2 top-1/2 z-50 max-h-[90dvh] w-[calc(100%_-_2rem)] max-w-2xl -translate-x-1/2 -translate-y-1/2 overflow-y-auto scroll-py-4 outline-none"
          onEscapeKeyDown={(event) => { if (isPending) event.preventDefault(); }}
          onPointerDownOutside={(event) => { if (isPending) event.preventDefault(); }}
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            const returnFocus = returnFocusRef.current;
            if (returnFocus?.isConnected) returnFocus.focus();
          }}
        >
        <div className="shrink-0 px-6 pt-6 sm:px-8 sm:pt-8">
          <Dialog.Title className="mb-2 text-2xl font-medium text-[#1f1f1f]">{t("Import novels")}</Dialog.Title>
          <Dialog.Description className="text-sm text-[#5f6368]">{t("Upload multiple files or paste the text of one novel.")}</Dialog.Description>
        </div>

        <form onSubmit={handleSubmit}>
          <fieldset disabled={isPending} className="space-y-5 px-6 py-6 sm:px-8">
            <div className="grid gap-4 sm:grid-cols-2">
              {isBatch ? (
                <div>
                  <p className="mb-1.5 text-sm font-medium text-[#3c4043]">{t("Title")}</p>
                  <div className="field-control flex items-center text-sm text-[#5f6368]">
                    {t("Use each file's name")}
                  </div>
                </div>
              ) : (
                <div>
                  <label htmlFor="novel-import-title" className="mb-1.5 block text-sm font-medium text-[#3c4043]">
                    {t("Title *")}
                  </label>
                  <input
                    id="novel-import-title"
                    value={title}
                    onChange={(event) => setTitle(event.target.value)}
                    placeholder={t("Enter a novel title")}
                    required
                    className="field-control text-sm"
                  />
                </div>
              )}
              <div>
                <label htmlFor="novel-import-author" className="mb-1.5 block text-sm font-medium text-[#3c4043]">{t("Author")}</label>
                <input
                  id="novel-import-author"
                  value={author}
                  onChange={(event) => setAuthor(event.target.value)}
                  placeholder={isBatch ? t("Optional, applies to all files") : t("Optional")}
                  className="field-control text-sm"
                />
              </div>
            </div>

            <div>
              <p className="mb-2 text-sm font-medium text-[#3c4043]">{t("Story deviation")}</p>
              <div className="grid gap-2 sm:grid-cols-3" role="group" aria-label={t("Story deviation")}>
                {[
                  { value: 'canon', label: t("Follow the original"), desc: t("Strictly follow the original") },
                  { value: 'creative', label: t("Creative expansion"), desc: t("Build on the original") },
                  { value: 'remix', label: t("Free rewriting"), desc: t("Change the story freely") },
                ].map((option) => (
                  <button
                    key={option.value}
                    type="button"
                    aria-pressed={deviationMode === option.value}
                    onClick={() => setDeviationMode(option.value)}
                    className="rounded-xl p-3 text-left transition-colors"
                    style={{
                      background: deviationMode === option.value ? '#e8f0fe' : '#fff',
                      border: `1px solid ${deviationMode === option.value ? '#0b57d0' : '#dadce0'}`,
                    }}
                  >
                    <span className="block text-xs font-semibold text-[#1f1f1f]">{option.label}</span>
                    <span className="mt-1 block text-xs text-[#5f6368]">{option.desc}</span>
                  </button>
                ))}
              </div>
            </div>

            <div>
              <p className="mb-2 text-sm font-medium text-[#3c4043]">{t("Novel files")}</p>
              <button
                type="button"
                onClick={() => fileInputRef.current?.click()}
                className="flex w-full items-center justify-center gap-2 rounded-xl px-4 py-6 text-sm transition-colors"
                style={{
                  background: files.length ? '#e6f4ea' : '#f8fafd',
                  border: `1px dashed ${files.length ? '#188038' : '#9aa0a6'}`,
                  color: files.length ? '#137333' : '#5f6368',
                }}
              >
                <Upload size={16} />
                {files.length ? t("{p0} novels selected", { p0: files.length }) : t("Choose TXT, EPUB or PDF files (multiple files allowed)")}
              </button>
              <input
                ref={fileInputRef}
                hidden
                type="file"
                multiple
                accept=".txt,.epub,.pdf,text/plain,application/epub+zip,application/pdf"
                onChange={(event) => {
                  selectFiles(Array.from(event.target.files ?? []));
                  event.currentTarget.value = '';
                }}
              />
              {files.length > 0 && (
                <ul aria-label={t("Selected novel files")} className="mt-2 space-y-1.5">
                  {files.map((file, index) => (
                    <li
                      key={`${file.name}-${file.size}-${file.lastModified}-${index}`}
                      className="flex items-center justify-between gap-3 rounded-lg bg-[#f8fafd] px-3 py-2 text-xs text-[#3c4043]"
                    >
                      <span className="min-w-0 truncate">{file.name}</span>
                      <button
                        type="button"
                        aria-label={t("Remove {p0}", { p0: file.name })}
                        onClick={() => removeFile(index)}
                        className="shrink-0 rounded-full p-1 text-[#5f6368] hover:bg-[#e8eaed]"
                      >
                        <X size={13} />
                      </button>
                    </li>
                  ))}
                </ul>
              )}
              <p className="mt-1.5 text-xs text-[#5f6368]">
                {t("Select up to 50 novels, uploaded in batches and queued for parsing. Each TXT can be up to 10 MiB; EPUB/PDF up to 20 MiB.")}
              </p>
            </div>

            <div className="flex items-center gap-3" aria-hidden="true">
              <div className="h-px flex-1 bg-[#dadce0]" />
              <span className="text-xs text-[#5f6368]">{t("Or paste one novel")}</span>
              <div className="h-px flex-1 bg-[#dadce0]" />
            </div>

            <div>
              <label htmlFor="novel-import-content" className="mb-1.5 block text-sm font-medium text-[#3c4043]">
                {t("Novel text")} {!files.length && '*'}
              </label>
              <textarea
                id="novel-import-content"
                value={content}
                onChange={(event) => {
                  setContent(event.target.value);
                  if (event.target.value) {
                    setFiles([]);
                    setTitle('');
                  }
                }}
                placeholder={t("Paste the novel text (Chinese or English; at least the first 3 chapters are recommended for character extraction)")}
                rows={6}
                required={!files.length}
                className="field-control max-h-[40dvh] resize-none text-sm"
                style={{ fontFamily: 'var(--font-reading)', lineHeight: '1.8' }}
              />
              <p className="mt-1 text-xs text-[#5f6368]">{t("Length:")}{content.length.toLocaleString(locale)} {t("characters")}</p>
            </div>

            <p className="rounded-xl border border-[#a8c7fa] bg-[#eef3fe] px-3 py-2.5 text-xs leading-5 text-[#174ea6]">
              {t("Accepted text and uploaded files when original-file storage is enabled remain with the shared original, including content still parsing or later failing. After parsing succeeds, other users can add it from the shared library. Your progress, identity, chats, memories and timeline remain private. Removing a book or deleting your account does not delete this shared content.")}
            </p>
            {unknownFiles.length > 0 && (
              <p role="alert" className="text-sm text-[#b3261e]">
                {t("Acceptance is unknown for these files. Check your shelf before selecting them again:")}{unknownFiles.join('、')}{t("The remaining list contains only files whose acceptance has not been confirmed.")}
              </p>
            )}
          </fieldset>

          <div className="flex shrink-0 justify-end gap-3 border-t border-[#e8eaed] px-6 py-4 sm:px-8">
            <Dialog.Close asChild>
              <button type="button" disabled={isPending} className="tonal-action text-sm">{t("Cancel")}</button>
            </Dialog.Close>
            <button
              type="submit"
              disabled={isPending || (!isBatch && !title.trim()) || (!files.length && !content.trim())}
              className="primary-action text-sm"
            >
              {isPending ? (
                <><Loader2 size={14} className="animate-spin" /> {t("Submitting…")}</>
              ) : (
                <><Upload size={14} /> {isBatch ? t("Import {p0} novels", { p0: files.length }) : t("Start import")}</>
              )}
            </button>
          </div>
        </form>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
