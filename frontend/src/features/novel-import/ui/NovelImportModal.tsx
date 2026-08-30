import { useRef, useState, type FormEvent } from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import { Loader2, Upload, X } from 'lucide-react';
import { toast } from 'sonner';
import {
  novelTitleFromFile,
  useImportNovel,
  useUploadNovel,
  useUploadNovelsBatch,
  validateNovelBatchFiles,
} from '@/entities/novel';
import { getApiErrorMessage } from '@/shared/api/client';

export function NovelImportModal({ onClose }: { onClose: () => void }) {
  const returnFocusRef = useRef<HTMLElement | null>(
    typeof document !== 'undefined' && document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null,
  );
  const [title, setTitle] = useState('');
  const [author, setAuthor] = useState('');
  const [content, setContent] = useState('');
  const [files, setFiles] = useState<File[]>([]);
  const [deviationMode, setDeviationMode] = useState('canon');
  const importNovel = useImportNovel();
  const uploadNovel = useUploadNovel();
  const uploadBatch = useUploadNovelsBatch();
  const isPending = importNovel.isPending || uploadNovel.isPending || uploadBatch.isPending;
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
      toast.success(isBatch ? `已开始导入 ${files.length} 本小说` : '小说导入已开始');
      onClose();
    } catch (error) {
      toast.error(getApiErrorMessage(error, isBatch ? '批量导入失败' : '小说导入失败'));
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
          className="surface-card fixed left-1/2 top-1/2 z-50 flex max-h-[90vh] w-[calc(100%_-_2rem)] max-w-2xl -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden outline-none"
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

        <form onSubmit={handleSubmit} className="flex min-h-0 flex-col">
          <div className="space-y-5 overflow-y-auto px-6 py-6 sm:px-8">
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
                  <label className="mb-1.5 block text-sm font-medium text-[#3c4043]">
                    书名 *
                  </label>
                  <input
                    value={title}
                    onChange={(event) => setTitle(event.target.value)}
                    placeholder="输入小说名称"
                    required
                    className="field-control text-sm"
                  />
                </div>
              )}
              <div>
                <label className="mb-1.5 block text-sm font-medium text-[#3c4043]">作者</label>
                <input
                  value={author}
                  onChange={(event) => setAuthor(event.target.value)}
                  placeholder={isBatch ? '可选，应用到全部文件' : '可选'}
                  className="field-control text-sm"
                />
              </div>
            </div>

            <div>
              <p className="mb-2 text-sm font-medium text-[#3c4043]">故事偏离度</p>
              <div className="grid gap-2 sm:grid-cols-3">
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
              <label
                className="flex w-full cursor-pointer items-center justify-center gap-2 rounded-xl px-4 py-6 text-sm transition-colors"
                style={{
                  background: files.length ? '#e6f4ea' : '#f8fafd',
                  border: `1px dashed ${files.length ? '#188038' : '#9aa0a6'}`,
                  color: files.length ? '#137333' : '#5f6368',
                }}
              >
                <Upload size={16} />
                {files.length ? `已选择 ${files.length} 本小说` : '选择 TXT、EPUB 或 PDF 文件（可多选）'}
                <input
                  type="file"
                  multiple
                  accept=".txt,.epub,.pdf,text/plain,application/epub+zip,application/pdf"
                  className="sr-only"
                  onChange={(event) => {
                    selectFiles(Array.from(event.target.files ?? []));
                    event.currentTarget.value = '';
                  }}
                />
              </label>
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
                每次最多 5 本、合计 40 MiB；单个 TXT 最大 10 MiB，EPUB/PDF 最大 20 MiB
              </p>
            </div>

            <div className="flex items-center gap-3" aria-hidden="true">
              <div className="h-px flex-1 bg-[#dadce0]" />
              <span className="text-xs text-[#5f6368]">或粘贴一本正文</span>
              <div className="h-px flex-1 bg-[#dadce0]" />
            </div>

            <div>
              <label className="mb-1.5 block text-sm font-medium text-[#3c4043]">
                小说内容 {!files.length && '*'}
              </label>
              <textarea
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
                className="field-control resize-none text-sm"
                style={{ fontFamily: 'var(--font-reading)', lineHeight: '1.8' }}
              />
              <p className="mt-1 text-xs text-[#5f6368]">字数：{content.length.toLocaleString()} 字</p>
            </div>
          </div>

          <div className="flex shrink-0 justify-end gap-3 border-t border-[#e8eaed] px-6 py-4 sm:px-8">
            <Dialog.Close asChild>
              <button type="button" className="tonal-action text-sm">取消</button>
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
