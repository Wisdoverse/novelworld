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
    localStorage.removeItem('auth_token');
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
    ['source_storage_unavailable', '已确认接收 0 本；另有 2 本结果未知，请先核对书架，避免重复上传。'],
    ['service_unavailable', '已确认接收 0 本；另有 2 本结果未知，请先核对书架，避免重复上传。'],
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

  it.each([422, 503])('stops after partial acceptance with status %s and preserves safe remaining files', async (status) => {
    const files = Array.from({ length: 12 }, (_, index) => new File(['text'], `${index}.txt`));
    const request = vi.spyOn(apiClient, 'post')
      .mockResolvedValueOnce({ data: { novels: files.slice(0, 5).map((_, index) => ({ novel_id: `id-${index}`, status: 'accepted' })) } })
      .mockRejectedValueOnce(new AxiosError('Upload failed', 'ERR_BAD_RESPONSE', undefined, undefined, {
        status, data: { error: { code: 'invalid_request', message: 'Invalid upload' } },
      } as AxiosResponse));
    const onClose = vi.fn();
    const queryClient = new QueryClient({ defaultOptions: { mutations: { retry: false }, queries: { retry: false } } });
    render(<QueryClientProvider client={queryClient}><NovelImportModal onClose={onClose} /></QueryClientProvider>);
    fireEvent.change(document.querySelector<HTMLInputElement>('input[type="file"]')!, { target: { files } });
    fireEvent.click(screen.getByRole('button', { name: '导入 12 本' }));
    await waitFor(() => expect(toast.error).toHaveBeenCalled());
    expect(request).toHaveBeenCalledTimes(2);
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.queryByText('0.txt')).toBeNull();
    if (status === 503) {
      expect(screen.getByRole('alert').textContent).toContain('5.txt');
      expect(screen.getByRole('button', { name: '导入 2 本' })).toBeTruthy();
    } else {
      expect(screen.queryByRole('alert')).toBeNull();
      expect(screen.getByRole('button', { name: '导入 7 本' })).toBeTruthy();
    }
  });

  it('uploads 50 files sequentially and locks the form throughout the sequence', async () => {
    const files = Array.from({ length: 50 }, (_, index) => new File(['text'], `${index}.txt`));
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    let active = 0;
    let maxActive = 0;
    const request = vi.spyOn(apiClient, 'post').mockImplementation(async (_url, data) => {
      active++;
      maxActive = Math.max(maxActive, active);
      await gate;
      active--;
      return { data: { novels: (data as FormData).getAll('file').map((file, index) => ({ novel_id: `${(file as File).name}-${index}`, status: 'accepted' })) } };
    });
    const onClose = vi.fn();
    const queryClient = new QueryClient({ defaultOptions: { mutations: { retry: false }, queries: { retry: false } } });
    render(<QueryClientProvider client={queryClient}><NovelImportModal onClose={onClose} /></QueryClientProvider>);
    fireEvent.change(document.querySelector<HTMLInputElement>('input[type="file"]')!, { target: { files } });
    fireEvent.click(screen.getByRole('button', { name: '导入 50 本' }));
    await waitFor(() => expect(request).toHaveBeenCalledOnce());
    expect((screen.getByRole('button', { name: '取消' }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByRole('button', { name: '移除 0.txt' }).matches(':disabled')).toBe(true);
    release();
    await waitFor(() => expect(onClose).toHaveBeenCalledOnce());
    expect(request).toHaveBeenCalledTimes(10);
    expect(maxActive).toBe(1);
    expect(request.mock.calls.flatMap(call => (call[1] as FormData).getAll('file'))).toEqual(files);
  });

  it('pins the initiating credentials and stops when the session changes between batches', async () => {
    localStorage.setItem('auth_token', 'token-A');
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const request = vi.spyOn(apiClient, 'post').mockImplementation(async (_url, data) => {
      await gate;
      return { data: { novels: (data as FormData).getAll('file').map((_, index) => ({ novel_id: `id-${index}`, status: 'accepted' })) } };
    });
    const onClose = vi.fn();
    const queryClient = new QueryClient({ defaultOptions: { mutations: { retry: false }, queries: { retry: false } } });
    render(<QueryClientProvider client={queryClient}><NovelImportModal onClose={onClose} /></QueryClientProvider>);
    fireEvent.change(document.querySelector<HTMLInputElement>('input[type="file"]')!, { target: {
      files: Array.from({ length: 6 }, (_, index) => new File(['text'], `${index}.txt`)),
    } });
    fireEvent.click(screen.getByRole('button', { name: '导入 6 本' }));
    await waitFor(() => expect(request).toHaveBeenCalledOnce());
    expect(request.mock.calls[0][2]?.headers).toEqual({ Authorization: 'Bearer token-A' });
    localStorage.setItem('auth_token', 'token-B');
    release();
    await waitFor(() => expect(onClose).toHaveBeenCalledOnce());
    expect(request).toHaveBeenCalledOnce();
    expect(toast.error).toHaveBeenCalledWith('登录状态已变化，已停止后续上传。请核对原账号书架。');
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
