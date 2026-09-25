import { useState } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AxiosError, type AxiosResponse } from 'axios';
import { toast } from 'sonner';
import { apiClient } from '@/shared/api/client';
import { NovelImportModal } from './NovelImportModal';

vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

function TestHost() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>打开导入</button>
      {open ? <NovelImportModal onClose={() => setOpen(false)} /> : null}
    </>
  );
}

describe('NovelImportModal', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    vi.clearAllMocks();
  });

  it('submits multiple selected files through the batch contract', async () => {
    const request = vi.spyOn(apiClient, 'post').mockResolvedValue({
      data: {
        novels: [
          { novel_id: 'first', status: 'parsing' },
          { novel_id: 'second', status: 'pending' },
        ],
      },
    });
    const onClose = vi.fn();
    const queryClient = new QueryClient({
      defaultOptions: { mutations: { retry: false }, queries: { retry: false } },
    });
    render(
      <QueryClientProvider client={queryClient}>
        <NovelImportModal onClose={onClose} />
      </QueryClientProvider>,
    );
    const input = document.querySelector<HTMLInputElement>('input[type="file"]');
    const files = [
      new File(['first'], 'first.txt', { type: 'text/plain' }),
      new File(['second'], 'second.pdf', { type: 'application/pdf' }),
    ];

    fireEvent.change(input!, { target: { files } });
    fireEvent.click(screen.getByRole('button', { name: '导入 2 本' }));

    await waitFor(() => expect(request).toHaveBeenCalledOnce());
    expect(request.mock.calls[0][0]).toBe('/novels/upload/batch');
    const form = request.mock.calls[0][1] as FormData;
    expect(form.getAll('file')).toEqual(files);
    expect(form.get('deviation_mode')).toBe('canon');
    expect(onClose).toHaveBeenCalledOnce();
  });

  it.each([
    ['import_capacity_busy', '解析任务繁忙，请稍后再导入。'],
    ['source_storage_unavailable', '文件存储暂时不可用，请稍后重试。'],
    ['service_unavailable', '导入服务暂时不可用，请稍后重试。'],
  ])('explains batch upload error %s', async (code, message) => {
    vi.spyOn(apiClient, 'post').mockRejectedValue(new AxiosError('Upload failed', 'ERR_BAD_RESPONSE', undefined, undefined, {
      status: 503,
      data: { error: { code, message: 'Service is temporarily unavailable' } },
    } as AxiosResponse));
    const queryClient = new QueryClient({
      defaultOptions: { mutations: { retry: false }, queries: { retry: false } },
    });
    render(
      <QueryClientProvider client={queryClient}>
        <NovelImportModal onClose={vi.fn()} />
      </QueryClientProvider>,
    );
    fireEvent.change(document.querySelector<HTMLInputElement>('input[type="file"]')!, {
      target: { files: [
        new File(['first'], 'first.txt', { type: 'text/plain' }),
        new File(['second'], 'second.txt', { type: 'text/plain' }),
      ] },
    });
    fireEvent.click(screen.getByRole('button', { name: '导入 2 本' }));
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith(message));
  });

  it('traps focus, closes with Escape, and restores focus to the opener', async () => {
    const queryClient = new QueryClient({
      defaultOptions: { mutations: { retry: false }, queries: { retry: false } },
    });
    render(
      <QueryClientProvider client={queryClient}>
        <TestHost />
      </QueryClientProvider>,
    );

    const opener = screen.getByRole('button', { name: '打开导入' });
    opener.focus();
    fireEvent.click(opener);
    const dialog = await screen.findByRole('dialog', { name: '导入小说' });
    const title = screen.getByPlaceholderText('输入小说名称');
    expect(screen.getByLabelText(/书名/)).toBe(title);
    expect(screen.getByLabelText('作者')).toBeTruthy();
    expect(screen.getByLabelText(/小说内容/)).toBeTruthy();
    expect(screen.getByRole('group', { name: '故事偏离度' })).toBeTruthy();
    expect(screen.getByText(/仍在解析或随后解析失败的内容/)).toBeTruthy();
    expect(screen.getByText(/删除账号不会删除这些共享内容/)).toBeTruthy();
    await waitFor(() => expect(document.activeElement).toBe(title));

    opener.focus();
    await waitFor(() => expect(dialog.contains(document.activeElement)).toBe(true));

    fireEvent.keyDown(document, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(opener));
  });
});
