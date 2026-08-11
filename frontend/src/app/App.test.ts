import { QueryClient } from '@tanstack/react-query';
import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';
import { useChatStore } from '@/features/character-chat/model/useChatStore';
import { apiClient } from '@/shared/api/client';
import { AppRoutes, resetPrivateClientStateForPrincipalChange } from './App';

describe('principal-scoped query cache', () => {
  it('clears cached tenant data before a different principal can use it', () => {
    const client = new QueryClient();
    const cancel = vi.fn();
    client.setQueryData(['reading-progress', 'novel'], { reader_identity: 'Alice' });
    useChatStore.setState({
      messages: {
        character: [{
          id: 'message',
          role: 'user',
          content: 'private',
          character_id: 'character',
          created_at: new Date(0).toISOString(),
        }],
      },
      cancelStream: { character: cancel },
    });
    const clear = vi.spyOn(client, 'clear');

    expect(resetPrivateClientStateForPrincipalChange(client, 'user-a', 'user-a')).toBe('user-a');
    expect(clear).not.toHaveBeenCalled();
    expect(resetPrivateClientStateForPrincipalChange(client, 'user-a', 'user-b')).toBe('user-b');
    expect(clear).toHaveBeenCalledOnce();
    expect(cancel).toHaveBeenCalledOnce();
    expect(client.getQueryData(['reading-progress', 'novel'])).toBeUndefined();
    expect(useChatStore.getState().messages).toEqual({});
  });
});

describe('setup status', () => {
  it('fails closed and offers a retry when server truth is unavailable', async () => {
    const request = vi
      .spyOn(apiClient, 'get')
      .mockRejectedValueOnce(new Error('offline'))
      .mockImplementationOnce(() => new Promise(() => undefined));
    render(React.createElement(MemoryRouter, null, React.createElement(AppRoutes)));

    expect((await screen.findByRole('alert')).textContent).toContain('Setup status unavailable');
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(request).toHaveBeenCalledTimes(2));
    request.mockRestore();
  });

  it('does not trust the retired in-memory setup contract during rollout', async () => {
    const request = vi.spyOn(apiClient, 'get').mockResolvedValueOnce({
      data: { configured: false },
    });
    render(React.createElement(MemoryRouter, null, React.createElement(AppRoutes)));

    expect((await screen.findByRole('alert')).textContent).toContain('Setup status unavailable');
    request.mockRestore();
  });
});
