import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { createChatStream, type ChatStreamError } from './client';

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
    localStorage.clear();
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
});
