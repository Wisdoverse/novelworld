import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { apiClient } from '@/shared/api/client';
import { NovelImportModal } from './NovelImportModal';

describe('NovelImportModal', () => {
  afterEach(() => vi.restoreAllMocks());

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
    const { container } = render(
      <QueryClientProvider client={queryClient}>
        <NovelImportModal onClose={onClose} />
      </QueryClientProvider>,
    );
    const input = container.querySelector<HTMLInputElement>('input[type="file"]');
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
});
