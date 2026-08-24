import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { queryClient } from './queryClient';
import { worldTurnPendingStorageKey } from '@/shared/lib/worldTurnStorage';
import {
  apiClient,
  createChatStream,
  invalidateSessionForUnauthorizedResponse,
  type ChatStreamError,
} from './client';

const encoder = new TextEncoder();

function responseFromChunks(
  chunks: Uint8Array[],
  status = 200,
  headers: Record<string, string> = {},
): Response {
  return new Response(new ReadableStream<Uint8Array>({
    start(controller) {
      chunks.forEach(chunk => controller.enqueue(chunk));
      controller.close();
    },
  }), {
    status,
    headers: {
      'Content-Type': status === 200 ? 'text/event-stream' : 'application/json',
      ...headers,
    },
  });
}

function streamChat() {
  const content: string[] = [];
  return new Promise<{ content: string; done: { turnId: string; replayed: boolean; legacy: boolean } }>((resolve, reject) => {
    createChatStream({
      characterId: 'character',
      turnId: '11111111-1111-4111-8111-111111111111',
      payload: {
        novel_id: 'novel',
        message: 'hello',
        current_chapter: 1,
      },
      onChunk: chunk => content.push(chunk),
      onDone: done => resolve({ content: content.join(''), done }),
      onError: error => reject(new Error(`${error.code}:${error.message}`)),
    });
  });
}

describe('createChatStream', () => {
  beforeEach(() => {
    queryClient.clear();
    localStorage.clear();
    sessionStorage.clear();
    vi.stubGlobal('fetch', vi.fn());
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('decodes v2 frames across every UTF-8 byte boundary and sends the turn only as a header', async () => {
    const turnId = '11111111-1111-4111-8111-111111111111';
    const sse = [
      'event: delta\r\n',
      'data: {"content":"你🙂"}\r\n\r\n',
      'event: done\n',
      `data: {"turn_id":"${turnId}","committed":true,"replayed":false}\n\n`,
    ].join('');
    const bytes = encoder.encode(sse);
    const fetchMock = vi.mocked(fetch).mockResolvedValue(
      responseFromChunks(Array.from(bytes, byte => Uint8Array.of(byte))),
    );

    const result = await streamChat();

    expect(result).toEqual({
      content: '你🙂',
      done: { turnId, replayed: false, legacy: false },
    });
    const init = fetchMock.mock.calls[0][1] as RequestInit;
    expect(init.headers).toMatchObject({ 'Idempotency-Key': turnId });
    expect(JSON.parse(init.body as string)).toEqual({
      novel_id: 'novel',
      message: 'hello',
      current_chapter: 1,
    });
    expect(init.body).not.toContain('turn_id');
  });

  it('supports legacy default events, mixed line endings, and multiple data lines', async () => {
    const sse = 'data: first\rdata: second\r\n\r\nevent: done\n\n';
    vi.mocked(fetch).mockResolvedValue(responseFromChunks([encoder.encode(sse)]));

    await expect(streamChat()).resolves.toEqual({
      content: 'first\nsecond',
      done: {
        turnId: '11111111-1111-4111-8111-111111111111',
        replayed: false,
        legacy: true,
      },
    });
  });

  it('fails a bare EOF instead of finalizing an uncommitted turn', async () => {
    vi.mocked(fetch).mockResolvedValue(
      responseFromChunks([encoder.encode('event: delta\ndata: {"content":"partial"}\n\n')]),
    );
    const onDone = vi.fn();
    const error = await new Promise<ChatStreamError>(resolve => {
      createChatStream({
        characterId: 'character',
        turnId: '11111111-1111-4111-8111-111111111111',
        payload: { novel_id: 'novel', message: 'hello', current_chapter: 1 },
        onChunk: vi.fn(),
        onDone,
        onError: resolve,
      });
    });

    expect(error.code).toBe('stream_incomplete');
    expect(onDone).not.toHaveBeenCalled();
  });

  it('fails invalid UTF-8 instead of replacing malformed bytes', async () => {
    vi.mocked(fetch).mockResolvedValue(responseFromChunks([Uint8Array.of(0xff)]));
    const onDone = vi.fn();
    const error = await new Promise<ChatStreamError>(resolve => {
      createChatStream({
        characterId: 'character',
        turnId: '11111111-1111-4111-8111-111111111111',
        payload: { novel_id: 'novel', message: 'hello', current_chapter: 1 },
        onChunk: vi.fn(),
        onDone,
        onError: resolve,
      });
    });

    expect(error.code).toBe('malformed_utf8');
    expect(onDone).not.toHaveBeenCalled();
  });

  it('retries a transient response with the same key and clears partial state first', async () => {
    vi.useFakeTimers();
    const turnId = '11111111-1111-4111-8111-111111111111';
    const done = `event: done\ndata: {"turn_id":"${turnId}","committed":true,"replayed":true}\n\n`;
    const fetchMock = vi.mocked(fetch)
      .mockResolvedValueOnce(responseFromChunks([encoder.encode('{}')], 503))
      .mockResolvedValueOnce(responseFromChunks([encoder.encode(done)]));
    const onRetry = vi.fn();
    const completed = new Promise<void>((resolve, reject) => {
      createChatStream({
        characterId: 'character',
        turnId,
        payload: { novel_id: 'novel', message: 'hello', current_chapter: 1 },
        onChunk: vi.fn(),
        onDone: () => resolve(),
        onError: error => reject(new Error(error.message)),
        onRetry,
      });
    });

    await vi.runAllTimersAsync();
    await completed;

    expect(onRetry).toHaveBeenCalledOnce();
    expect(fetchMock).toHaveBeenCalledTimes(2);
    for (const [, init] of fetchMock.mock.calls) {
      expect(init?.headers).toMatchObject({ 'Idempotency-Key': turnId });
    }
  });

  it('waits for the server lease before retrying an in-progress turn', async () => {
    vi.useFakeTimers();
    const turnId = '11111111-1111-4111-8111-111111111111';
    const done = `event: done\ndata: {"turn_id":"${turnId}","committed":true,"replayed":true}\n\n`;
    const fetchMock = vi.mocked(fetch)
      .mockResolvedValueOnce(responseFromChunks(
        [encoder.encode('{"error":{"code":"turn_in_progress"}}')],
        409,
        { 'Retry-After': '120' },
      ))
      .mockResolvedValueOnce(responseFromChunks([encoder.encode(done)]));
    const completed = streamChat();

    await vi.advanceTimersByTimeAsync(119_999);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);

    await expect(completed).resolves.toMatchObject({ done: { turnId, replayed: true } });
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it('invalidates the current principal when the SSE request gets a same-token 401', async () => {
    localStorage.setItem('auth_token', 'access');
    localStorage.setItem('refresh_token', 'refresh');
    sessionStorage.setItem(worldTurnPendingStorageKey('user-a', 'novel-a'), 'pending-a');
    queryClient.setQueryData(['principal-a'], 'private-a');
    vi.mocked(fetch).mockResolvedValue(
      responseFromChunks([encoder.encode('{"error":{"code":"unauthorized"}}')], 401),
    );

    const error = await new Promise<ChatStreamError>(resolve => {
      createChatStream({
        characterId: 'character',
        turnId: '11111111-1111-4111-8111-111111111111',
        payload: { novel_id: 'novel', message: 'hello', current_chapter: 1 },
        onChunk: vi.fn(),
        onDone: vi.fn(),
        onError: resolve,
      });
    });

    expect(error.code).toBe('unauthorized');
    expect(localStorage.getItem('auth_token')).toBeNull();
    expect(localStorage.getItem('refresh_token')).toBeNull();
    expect(sessionStorage.getItem(worldTurnPendingStorageKey('user-a', 'novel-a'))).toBeNull();
    expect(queryClient.getQueryData(['principal-a'])).toBeUndefined();
  });

  it('does not invalidate B when an old-token SSE request gets a late 401', async () => {
    localStorage.setItem('auth_token', 'access-a');
    vi.mocked(fetch).mockResolvedValue(
      responseFromChunks([encoder.encode('{"error":{"code":"unauthorized"}}')], 401),
    );
    const error = new Promise<ChatStreamError>(resolve => {
      createChatStream({
        characterId: 'character',
        turnId: '11111111-1111-4111-8111-111111111111',
        payload: { novel_id: 'novel', message: 'hello', current_chapter: 1 },
        onChunk: vi.fn(),
        onDone: vi.fn(),
        onError: resolve,
      });
    });
    localStorage.setItem('auth_token', 'access-b');
    localStorage.setItem('refresh_token', 'refresh-b');
    sessionStorage.setItem(worldTurnPendingStorageKey('user-b', 'novel-b'), 'pending-b');
    queryClient.setQueryData(['principal-b'], 'private-b');

    await expect(error).resolves.toMatchObject({ code: 'unauthorized' });
    expect(localStorage.getItem('auth_token')).toBe('access-b');
    expect(localStorage.getItem('refresh_token')).toBe('refresh-b');
    expect(sessionStorage.getItem(worldTurnPendingStorageKey('user-b', 'novel-b'))).toBe('pending-b');
    expect(queryClient.getQueryData(['principal-b'])).toBe('private-b');
  });

  it('clears all private client state after a protected API returns 401', () => {
    localStorage.setItem('auth_token', 'access');
    localStorage.setItem('refresh_token', 'refresh');
    sessionStorage.setItem(worldTurnPendingStorageKey('user-a', 'novel-a'), 'private');
    sessionStorage.setItem('unrelated', 'keep');
    queryClient.setQueryData(['private'], 'marker');

    expect(invalidateSessionForUnauthorizedResponse({
      isAxiosError: true,
      config: { url: '/novels', headers: { Authorization: 'Bearer access' } },
      response: { status: 401 },
    })).toBe(true);

    expect(localStorage.getItem('auth_token')).toBeNull();
    expect(localStorage.getItem('refresh_token')).toBeNull();
    expect(sessionStorage.getItem(worldTurnPendingStorageKey('user-a', 'novel-a'))).toBeNull();
    expect(sessionStorage.getItem('unrelated')).toBe('keep');
    expect(queryClient.getQueryData(['private'])).toBeUndefined();
  });

  it('preserves a newer principal when an older bearer request returns 401 late', () => {
    localStorage.setItem('auth_token', 'new-access');
    localStorage.setItem('refresh_token', 'new-refresh');
    sessionStorage.setItem(worldTurnPendingStorageKey('user-b', 'novel-b'), 'pending-b');
    queryClient.setQueryData(['principal-b'], 'private-b');

    expect(invalidateSessionForUnauthorizedResponse({
      isAxiosError: true,
      config: { url: '/novels', headers: { Authorization: 'Bearer old-access' } },
      response: { status: 401 },
    })).toBe(false);

    expect(localStorage.getItem('auth_token')).toBe('new-access');
    expect(localStorage.getItem('refresh_token')).toBe('new-refresh');
    expect(sessionStorage.getItem(worldTurnPendingStorageKey('user-b', 'novel-b'))).toBe('pending-b');
    expect(queryClient.getQueryData(['principal-b'])).toBe('private-b');
  });

  it.each(['/auth/login', '/auth/register', '/setup/init'])(
    'preserves the current principal after a public credential failure at %s',
    requestUrl => {
      localStorage.setItem('auth_token', 'access');
      localStorage.setItem('refresh_token', 'refresh');
      sessionStorage.setItem(worldTurnPendingStorageKey('user-a', 'novel-a'), 'private');
      queryClient.setQueryData(['private'], 'marker');

      expect(invalidateSessionForUnauthorizedResponse({
        isAxiosError: true,
        config: { url: requestUrl },
        response: { status: 401 },
      })).toBe(false);

      expect(localStorage.getItem('auth_token')).toBe('access');
      expect(localStorage.getItem('refresh_token')).toBe('refresh');
      expect(sessionStorage.getItem(worldTurnPendingStorageKey('user-a', 'novel-a'))).toBe('private');
      expect(queryClient.getQueryData(['private'])).toBe('marker');
    },
  );

  it('does not treat a general 403 as global session invalidation', () => {
    localStorage.setItem('auth_token', 'access');
    queryClient.setQueryData(['private'], 'marker');

    expect(invalidateSessionForUnauthorizedResponse({
      isAxiosError: true,
      config: { url: '/novels' },
      response: { status: 403 },
    })).toBe(false);

    expect(localStorage.getItem('auth_token')).toBe('access');
    expect(queryClient.getQueryData(['private'])).toBe('marker');
  });

  it('preserves an explicit initiating credential when the current principal changes', async () => {
    localStorage.setItem('auth_token', 'access-b');
    let authorization: unknown;

    await apiClient.post('/auth/logout', { refresh_token: 'refresh-a' }, {
      headers: { Authorization: 'Bearer access-a' },
      adapter: async config => {
        authorization = config.headers.get('Authorization');
        return {
          data: undefined,
          status: 200,
          statusText: 'OK',
          headers: {},
          config,
        };
      },
    });

    expect(authorization).toBe('Bearer access-a');
  });
});
