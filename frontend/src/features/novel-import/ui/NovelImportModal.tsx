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
      toast.success(isBatch ? `已接收 ${files.length} 本小说，等待解析` : '小说已接收，等待解析');
      onClose();
    } catch (error) {
      if (error instanceof NovelBatchUploadError) {
        if (error.reason === 'session_changed') {
          toast.error('登录状态已变化，已停止后续上传。请核对原账号书架。');
          onClose();
          return;
        }
        setFiles(error.remainingFiles);
        setTitle(error.remainingFiles.length === 1 ? novelTitleFromFile(error.remainingFiles[0]) : '');
        if (error.unknownFiles.length) {
          setUnknownFiles(error.unknownFiles.map(file => file.name));
          toast.error(`已确认接收 ${error.accepted.length} 本；另有 ${error.unknownFiles.length} 本结果未知，请先核对书架，避免重复上传。`);
          return;
        }
        const prefix = error.accepted.length ? `已接收 ${error.accepted.length} 本；剩余文件未接收。` : '';
        toast.error(prefix + (getApiErrorCode(error.cause) === 'upload_capacity_busy'
          ? '上传繁忙，请稍后重试。' : getApiErrorMessage(error.cause, '上传失败，请检查剩余文件后重试。')));
        return;
      }
      const code = getApiErrorCode(error);
      const message = code === 'upload_capacity_busy'
        ? '上传繁忙，请稍后重试。'
        : code === 'import_capacity_busy'
        ? '解析任务繁忙，请稍后再导入。'
        : code === 'source_storage_unavailable'
          ? '文件存储暂时不可用，请稍后重试。'
          : code === 'service_unavailable' || code === 'bad_gateway'
            ? '导入服务暂时不可用，请稍后重试。'
            : getApiErrorMessage(error, isBatch ? '批量导入失败' : '小说导入失败');
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
          <Dialog.Title className="mb-2 text-2xl font-medium text-[#1f1f1f]">导入小说</Dialog.Title>
          <Dialog.Description className="text-sm text-[#5f6368]">可批量上传文件，或粘贴一本小说的正文。</Dialog.Description>
        </div>

        <form onSubmit={handleSubmit}>
          <fieldset disabled={isPending} className="space-y-5 px-6 py-6 sm:px-8">
            <div className="grid gap-4 sm:grid-cols-2">
              {isBatch ? (
                <div>
                  <p className="mb-1.5 text-sm font-medium text-[#3c4043]">书名</p>
                  <div className="field-control flex items-center text-sm text-[#5f6368]">
                    使用每个文件的文件名
                  </div>
                </div>
              ) : (
                <div>
                  <label htmlFor="novel-import-title" className="mb-1.5 block text-sm font-medium text-[#3c4043]">
                    书名 *
                  </label>
                  <input
                    id="novel-import-title"
                    value={title}
                    onChange={(event) => setTitle(event.target.value)}
                    placeholder="输入小说名称"
                    required
                    className="field-control text-sm"
                  />
                </div>
              )}
              <div>
                <label htmlFor="novel-import-author" className="mb-1.5 block text-sm font-medium text-[#3c4043]">作者</label>
                <input
                  id="novel-import-author"
                  value={author}
                  onChange={(event) => setAuthor(event.target.value)}
                  placeholder={isBatch ? '可选，应用到全部文件' : '可选'}
                  className="field-control text-sm"
                />
              </div>
            </div>

            <div>
              <p className="mb-2 text-sm font-medium text-[#3c4043]">故事偏离度</p>
              <div className="grid gap-2 sm:grid-cols-3" role="group" aria-label="故事偏离度">
                {[
                  { value: 'canon', label: '忠实原著', desc: '严格遵循原著' },
                  { value: 'creative', label: '创意扩展', desc: '在原著基础上发挥' },
                  { value: 'remix', label: '自由改写', desc: '大胆改变走向' },
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
              <p className="mb-2 text-sm font-medium text-[#3c4043]">小说文件</p>
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
                {files.length ? `已选择 ${files.length} 本小说` : '选择 TXT、EPUB 或 PDF 文件（可多选）'}
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
                <ul aria-label="已选择的小说文件" className="mt-2 space-y-1.5">
                  {files.map((file, index) => (
                    <li
                      key={`${file.name}-${file.size}-${file.lastModified}-${index}`}
                      className="flex items-center justify-between gap-3 rounded-lg bg-[#f8fafd] px-3 py-2 text-xs text-[#3c4043]"
                    >
                      <span className="min-w-0 truncate">{file.name}</span>
                      <button
                        type="button"
                        aria-label={`移除 ${file.name}`}
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
                每次可选 50 本，自动分批上传并排队解析；单个 TXT 最大 10 MiB，EPUB/PDF 最大 20 MiB
              </p>
            </div>

            <div className="flex items-center gap-3" aria-hidden="true">
              <div className="h-px flex-1 bg-[#dadce0]" />
              <span className="text-xs text-[#5f6368]">或粘贴一本正文</span>
              <div className="h-px flex-1 bg-[#dadce0]" />
            </div>

            <div>
              <label htmlFor="novel-import-content" className="mb-1.5 block text-sm font-medium text-[#3c4043]">
                小说内容 {!files.length && '*'}
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
                placeholder="粘贴小说全文内容（支持中英文，建议至少粘贴前3章用于角色提取）"
                rows={6}
                required={!files.length}
                className="field-control max-h-[40dvh] resize-none text-sm"
                style={{ fontFamily: 'var(--font-reading)', lineHeight: '1.8' }}
              />
              <p className="mt-1 text-xs text-[#5f6368]">字数：{content.length.toLocaleString()} 字</p>
            </div>

            <p className="rounded-xl border border-[#a8c7fa] bg-[#eef3fe] px-3 py-2.5 text-xs leading-5 text-[#174ea6]">
              提交并被系统接受的正文，以及启用原文件存储时的上传文件，包括仍在解析或随后解析失败的内容，都会随共享原著保留。解析成功后其他用户可从共享书库加入；你的阅读进度、身份、对话、记忆和时间线仍为私有。移出书架或删除账号不会删除这些共享内容。
            </p>
            {unknownFiles.length > 0 && (
              <p role="alert" className="text-sm text-[#b3261e]">
                以下文件的接收结果未知，请先核对书架再决定是否重新选择：{unknownFiles.join('、')}。
                剩余列表仅包含尚未确认接收的文件。
              </p>
            )}
          </fieldset>

          <div className="flex shrink-0 justify-end gap-3 border-t border-[#e8eaed] px-6 py-4 sm:px-8">
            <Dialog.Close asChild>
              <button type="button" disabled={isPending} className="tonal-action text-sm">取消</button>
            </Dialog.Close>
            <button
              type="submit"
              disabled={isPending || (!isBatch && !title.trim()) || (!files.length && !content.trim())}
              className="primary-action text-sm"
            >
              {isPending ? (
                <><Loader2 size={14} className="animate-spin" /> 提交中...</>
              ) : (
                <><Upload size={14} /> {isBatch ? `导入 ${files.length} 本` : '开始导入'}</>
              )}
            </button>
          </div>
        </form>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
