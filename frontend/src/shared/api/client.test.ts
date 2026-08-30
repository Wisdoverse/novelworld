import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { queryClient } from './queryClient';
import { worldTurnPendingStorageKey } from '@/shared/lib/worldTurnStorage';
import {
  apiClient,
  createChatStream,
  invalidateSessionForUnauthorizedResponse,
  refreshSessionForAccessToken,
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
    vi.restoreAllMocks();
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
    vi.spyOn(apiClient, 'post').mockRejectedValueOnce({
      isAxiosError: true,
      response: { status: 401 },
    });
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

  it('preserves the current principal when SSE session refresh is temporarily unavailable', async () => {
    localStorage.setItem('auth_token', 'access');
    localStorage.setItem('refresh_token', 'refresh');
    sessionStorage.setItem(worldTurnPendingStorageKey('user-a', 'novel-a'), 'pending-a');
    queryClient.setQueryData(['principal-a'], 'private-a');
    vi.spyOn(apiClient, 'post').mockRejectedValueOnce({
      isAxiosError: true,
      response: { status: 503 },
    });
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

    expect(error.code).toBe('session_refresh_unavailable');
    expect(localStorage.getItem('auth_token')).toBe('access');
    expect(localStorage.getItem('refresh_token')).toBe('refresh');
    expect(sessionStorage.getItem(worldTurnPendingStorageKey('user-a', 'novel-a'))).toBe('pending-a');
    expect(queryClient.getQueryData(['principal-a'])).toBe('private-a');
  });

  it('rotates once and replays a protected Axios request with the new access token', async () => {
    localStorage.setItem('auth_token', 'old-access');
    localStorage.setItem('refresh_token', 'old-refresh');
    const refresh = vi.spyOn(apiClient, 'post').mockResolvedValueOnce({
      data: { access_token: 'new-access', refresh_token: 'new-refresh' },
    });
    const attempts: string[] = [];

    const response = await apiClient.get('/novels', {
      adapter: async config => {
        const authorization = String(config.headers.get('Authorization'));
        attempts.push(authorization);
        if (authorization === 'Bearer old-access') {
          throw Object.assign(new Error('Unauthorized'), {
            isAxiosError: true,
            config,
            response: { status: 401, data: {}, headers: {}, config },
          });
        }
        return { data: ['replayed'], status: 200, statusText: 'OK', headers: {}, config };
      },
    });

    expect(response.data).toEqual(['replayed']);
    expect(attempts).toEqual(['Bearer old-access', 'Bearer new-access']);
    expect(refresh).toHaveBeenCalledOnce();
    expect(refresh).toHaveBeenCalledWith('/auth/refresh', { refresh_token: 'old-refresh' });
    expect(localStorage.getItem('auth_token')).toBe('new-access');
    expect(localStorage.getItem('refresh_token')).toBe('new-refresh');
  });

  it('preserves the current principal when Axios session refresh is temporarily unavailable', async () => {
    localStorage.setItem('auth_token', 'access');
    localStorage.setItem('refresh_token', 'refresh');
    sessionStorage.setItem(worldTurnPendingStorageKey('user-a', 'novel-a'), 'pending-a');
    queryClient.setQueryData(['principal-a'], 'private-a');
    const refreshError = {
      isAxiosError: true,
      response: { status: 503 },
    };
    vi.spyOn(apiClient, 'post').mockRejectedValueOnce(refreshError);

    const request = apiClient.get('/novels', {
      adapter: async config => {
        throw Object.assign(new Error('Unauthorized'), {
          isAxiosError: true,
          config,
          response: { status: 401, data: {}, headers: {}, config },
        });
      },
    });

    await expect(request).rejects.toBe(refreshError);
    expect(localStorage.getItem('auth_token')).toBe('access');
    expect(localStorage.getItem('refresh_token')).toBe('refresh');
    expect(sessionStorage.getItem(worldTurnPendingStorageKey('user-a', 'novel-a'))).toBe('pending-a');
    expect(queryClient.getQueryData(['principal-a'])).toBe('private-a');
  });

  it('shares one refresh rotation across concurrent 401 responses', async () => {
    localStorage.setItem('auth_token', 'concurrent-access');
    localStorage.setItem('refresh_token', 'concurrent-refresh');
    let resolveRefresh!: (value: { data: { access_token: string; refresh_token: string } }) => void;
    const refresh = vi.spyOn(apiClient, 'post').mockImplementationOnce(
      () => new Promise(resolve => { resolveRefresh = resolve; }),
    );

    const first = refreshSessionForAccessToken('concurrent-access');
    const second = refreshSessionForAccessToken('concurrent-access');
    expect(refresh).toHaveBeenCalledOnce();
    resolveRefresh({ data: { access_token: 'rotated-access', refresh_token: 'rotated-refresh' } });

    await expect(Promise.all([first, second])).resolves.toEqual([
      'rotated-access',
      'rotated-access',
    ]);
    expect(localStorage.getItem('refresh_token')).toBe('rotated-refresh');
  });

  it('does not let a late refresh response overwrite a newer principal', async () => {
    localStorage.setItem('auth_token', 'access-a');
    localStorage.setItem('refresh_token', 'refresh-a');
    let resolveRefresh!: (value: { data: { access_token: string; refresh_token: string } }) => void;
    vi.spyOn(apiClient, 'post').mockImplementationOnce(
      () => new Promise(resolve => { resolveRefresh = resolve; }),
    );

    const refresh = refreshSessionForAccessToken('access-a');
    localStorage.setItem('auth_token', 'access-b');
    localStorage.setItem('refresh_token', 'refresh-b');
    resolveRefresh({ data: { access_token: 'late-a', refresh_token: 'late-refresh-a' } });

    await expect(refresh).rejects.toThrow(/principal changed/i);
    expect(localStorage.getItem('auth_token')).toBe('access-b');
    expect(localStorage.getItem('refresh_token')).toBe('refresh-b');
  });

  it('refreshes and retries a POST SSE turn with the same idempotency key', async () => {
    localStorage.setItem('auth_token', 'expired-access');
    localStorage.setItem('refresh_token', 'valid-refresh');
    vi.spyOn(apiClient, 'post').mockResolvedValueOnce({
      data: { access_token: 'fresh-access', refresh_token: 'rotated-refresh' },
    });
    const turnId = '11111111-1111-4111-8111-111111111111';
    const done = `event: done\ndata: {"turn_id":"${turnId}","committed":true,"replayed":true}\n\n`;
    const fetchMock = vi.mocked(fetch)
      .mockResolvedValueOnce(responseFromChunks([encoder.encode('{}')], 401))
      .mockResolvedValueOnce(responseFromChunks([encoder.encode(done)]));

    await expect(streamChat()).resolves.toMatchObject({ done: { turnId, replayed: true } });
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect((fetchMock.mock.calls[0][1]?.headers as Record<string, string>).Authorization)
      .toBe('Bearer expired-access');
    expect((fetchMock.mock.calls[1][1]?.headers as Record<string, string>).Authorization)
      .toBe('Bearer fresh-access');
    for (const [, init] of fetchMock.mock.calls) {
      expect(init?.headers).toMatchObject({ 'Idempotency-Key': turnId });
    }
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
