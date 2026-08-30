import axios, { AxiosHeaders, AxiosInstance, type InternalAxiosRequestConfig } from 'axios';
import { isDesktopClient } from '@/shared/config/runtime';
import { clearPrivateQueryCache } from '@/shared/api/queryClient';
import { clearWorldTurnPendingRequests } from '@/shared/lib/worldTurnStorage';

const BASE_URL = import.meta.env.VITE_API_URL || 'http://localhost:8080/api';

export const apiClient: AxiosInstance = axios.create({
  baseURL: BASE_URL,
  timeout: 30000,
});

const PUBLIC_CREDENTIAL_PATHS = new Set([
  '/auth/login',
  '/auth/logout',
  '/auth/refresh',
  '/auth/register',
  '/setup/init',
]);

type RefreshableRequestConfig = InternalAxiosRequestConfig & {
  _sessionRefreshAttempted?: boolean;
};

type RefreshResult = {
  previousAccessToken: string;
  accessToken: string;
  refreshToken: string;
};

let refreshFlight: { refreshToken: string; promise: Promise<RefreshResult> } | undefined;
let lastRefresh: RefreshResult | undefined;

class StaleSessionError extends Error {
  constructor() {
    super('The authenticated principal changed while refreshing the session');
  }
}

function isDefinitiveRefreshRejection(error: unknown) {
  if (!axios.isAxiosError(error)) return false;
  return error.response?.status === 401 || error.response?.status === 403;
}

function requestPath(url?: string) {
  if (!url) return undefined;
  try {
    const parsed = new URL(url, BASE_URL);
    return parsed.pathname.replace(/^\/api(?=\/)/, '');
  } catch {
    return url.split(/[?#]/)[0];
  }
}

function bearerToken(value: unknown) {
  return typeof value === 'string' && value.startsWith('Bearer ')
    ? value.slice('Bearer '.length)
    : undefined;
}

function clearSession(expectedAccessToken: string, expectedRefreshToken?: string) {
  if (localStorage.getItem('auth_token') !== expectedAccessToken) return false;
  if (
    expectedRefreshToken !== undefined
    && localStorage.getItem('refresh_token') !== expectedRefreshToken
  ) return false;
  clearPrivateQueryCache();
  localStorage.removeItem('auth_token');
  localStorage.removeItem('refresh_token');
  clearWorldTurnPendingRequests();
  return true;
}

/** Rotate one refresh token at most once, even when several requests fail together. */
export async function refreshSessionForAccessToken(
  previousAccessToken: string,
): Promise<string> {
  const currentAccessToken = localStorage.getItem('auth_token');
  const currentRefreshToken = localStorage.getItem('refresh_token');

  if (
    lastRefresh?.previousAccessToken === previousAccessToken
    && lastRefresh.accessToken === currentAccessToken
    && lastRefresh.refreshToken === currentRefreshToken
  ) return lastRefresh.accessToken;

  if (currentAccessToken !== previousAccessToken || !currentRefreshToken) {
    throw new StaleSessionError();
  }

  if (refreshFlight?.refreshToken === currentRefreshToken) {
    return (await refreshFlight.promise).accessToken;
  }

  const previousRefreshToken = currentRefreshToken;
  const promise = apiClient.post<{ access_token: string; refresh_token: string }>(
    '/auth/refresh',
    { refresh_token: previousRefreshToken },
  ).then(({ data }) => {
    if (
      localStorage.getItem('auth_token') !== previousAccessToken
      || localStorage.getItem('refresh_token') !== previousRefreshToken
    ) throw new StaleSessionError();

    const result: RefreshResult = {
      previousAccessToken,
      accessToken: data.access_token,
      refreshToken: data.refresh_token,
    };
    localStorage.setItem('auth_token', result.accessToken);
    localStorage.setItem('refresh_token', result.refreshToken);
    lastRefresh = result;
    return result;
  }).finally(() => {
    if (refreshFlight?.promise === promise) refreshFlight = undefined;
  });
  refreshFlight = { refreshToken: previousRefreshToken, promise };
  return (await promise).accessToken;
}

export function invalidateSessionForUnauthorizedRequest(
  status: number | undefined,
  requestAuthorization: unknown,
  requestUrl?: string,
) {
  if (status !== 401) return false;
  if (requestUrl && PUBLIC_CREDENTIAL_PATHS.has(requestPath(requestUrl) ?? requestUrl)) return false;
  const currentToken = localStorage.getItem('auth_token');
  if (
    !currentToken
    || typeof requestAuthorization !== 'string'
    || requestAuthorization !== `Bearer ${currentToken}`
  ) return false;
  return clearSession(currentToken);
}

export function invalidateSessionForUnauthorizedResponse(error: unknown) {
  if (!axios.isAxiosError(error)) return false;
  return invalidateSessionForUnauthorizedRequest(
    error.response?.status,
    AxiosHeaders.from(error.config?.headers).get('Authorization'),
    requestPath(error.config?.url),
  );
}

function reloadOrRedirectAfterUnauthorized() {
  if (isDesktopClient) {
    window.location.reload();
  } else {
    window.location.href = '/login';
  }
}

interface ApiErrorBody {
  error?: string | { code?: string; message?: string };
}

export function getApiErrorCode(error: unknown): string | undefined {
  if (!axios.isAxiosError<ApiErrorBody>(error)) return undefined;
  const detail = error.response?.data?.error;
  return typeof detail === 'string' ? undefined : detail?.code;
}

export function getApiErrorMessage(error: unknown, fallback: string): string {
  if (!axios.isAxiosError<ApiErrorBody>(error)) return fallback;
  const detail = error.response?.data?.error;
  return typeof detail === 'string' ? detail : detail?.message || fallback;
}

// 请求拦截器：注入 JWT
apiClient.interceptors.request.use((config) => {
  const token = localStorage.getItem('auth_token');
  if (token && !config.headers.has('Authorization')) {
    config.headers.Authorization = `Bearer ${token}`;
  }
  return config;
});

// 响应拦截器：统一错误处理
apiClient.interceptors.response.use(
  (response) => response,
  async (error) => {
    if (!axios.isAxiosError(error)) return Promise.reject(error);
    const config = error.config as RefreshableRequestConfig | undefined;
    const path = requestPath(config?.url);
    const authorization = AxiosHeaders.from(config?.headers).get('Authorization');
    const failedAccessToken = bearerToken(authorization);
    const canRefresh = error.response?.status === 401
      && config !== undefined
      && !config._sessionRefreshAttempted
      && (!path || !PUBLIC_CREDENTIAL_PATHS.has(path))
      && failedAccessToken !== undefined;

    if (canRefresh) {
      config._sessionRefreshAttempted = true;
      try {
        const accessToken = await refreshSessionForAccessToken(failedAccessToken);
        config.headers = AxiosHeaders.from(config.headers);
        config.headers.set('Authorization', `Bearer ${accessToken}`);
        return await apiClient.request(config);
      } catch (refreshError) {
        if (!isDefinitiveRefreshRejection(refreshError)) {
          return Promise.reject(refreshError);
        }
        // The failing request is invalidated only when it still belongs to the
        // current principal. A late response from an older principal is inert.
      }
    }

    if (invalidateSessionForUnauthorizedResponse(error)) {
      reloadOrRedirectAfterUnauthorized();
    }
    return Promise.reject(error);
  }
);

export interface ChatStreamPayload {
  novel_id: string;
  message: string;
  reader_identity?: string;
  current_chapter: number;
}

export interface ChatStreamDone {
  turnId: string;
  replayed: boolean;
  legacy: boolean;
}

export interface ChatStreamError {
  code: string;
  message: string;
}

export interface ChatStreamOptions {
  characterId: string;
  turnId: string;
  payload: ChatStreamPayload;
  onChunk: (text: string) => void;
  onDone: (result: ChatStreamDone) => void;
  onError: (error: ChatStreamError) => void;
  onRetry?: () => void;
}

class ChatProtocolError extends Error {
  constructor(readonly code: string, message: string) {
    super(message);
  }
}

interface ChatSseCallbacks {
  onChunk: (text: string) => void;
  onDone: (result: ChatStreamDone) => void;
  onError: (error: ChatStreamError) => void;
}

class ChatSseParser {
  private buffer = '';
  private eventName = '';
  private dataLines: string[] = [];
  private sawV2 = false;
  private terminal = false;

  constructor(
    private readonly turnId: string,
    private readonly callbacks: ChatSseCallbacks,
  ) {}

  get isTerminal() {
    return this.terminal;
  }

  feed(text: string) {
    if (this.terminal || !text) return;
    this.buffer += text;
    let consumed = 0;

    for (let index = 0; index < this.buffer.length;) {
      const character = this.buffer[index];
      if (character !== '\n' && character !== '\r') {
        index++;
        continue;
      }
      if (character === '\r' && index + 1 === this.buffer.length) break;

      this.processLine(this.buffer.slice(consumed, index));
      index += character === '\r' && this.buffer[index + 1] === '\n' ? 2 : 1;
      consumed = index;
      if (this.terminal) break;
    }

    this.buffer = this.buffer.slice(consumed);
  }

  finish() {
    if (!this.terminal) {
      throw new ChatProtocolError(
        'stream_incomplete',
        'The response ended before the turn was committed',
      );
    }
  }

  private processLine(line: string) {
    if (line === '') {
      this.dispatch();
      return;
    }
    if (line.startsWith(':')) return;

    const colon = line.indexOf(':');
    const field = colon === -1 ? line : line.slice(0, colon);
    let value = colon === -1 ? '' : line.slice(colon + 1);
    if (value.startsWith(' ')) value = value.slice(1);

    if (field === 'event') this.eventName = value;
    if (field === 'data') this.dataLines.push(value);
  }

  private dispatch() {
    const eventName = this.eventName || 'message';
    const data = this.dataLines.join('\n');
    this.eventName = '';
    this.dataLines = [];

    if (eventName === 'message') {
      if (data) this.callbacks.onChunk(data);
      return;
    }

    if (eventName === 'delta') {
      this.sawV2 = true;
      const event = this.parseObject(data, 'malformed_delta');
      if (typeof event.content !== 'string') {
        throw new ChatProtocolError('malformed_delta', 'Delta content must be a string');
      }
      this.callbacks.onChunk(event.content);
      return;
    }

    if (eventName === 'done') {
      if (!data && !this.sawV2) {
        this.terminal = true;
        this.callbacks.onDone({ turnId: this.turnId, replayed: false, legacy: true });
        return;
      }

      const event = this.parseObject(data, 'malformed_done');
      if (
        event.turn_id !== this.turnId
        || event.committed !== true
        || (event.replayed !== undefined && typeof event.replayed !== 'boolean')
      ) {
        throw new ChatProtocolError('malformed_done', 'Commit acknowledgement is invalid');
      }
      this.terminal = true;
      this.callbacks.onDone({
        turnId: this.turnId,
        replayed: event.replayed === true,
        legacy: false,
      });
      return;
    }

    if (eventName === 'error') {
      this.terminal = true;
      if (!data) {
        this.callbacks.onError({ code: 'stream_error', message: 'Response generation failed' });
        return;
      }
      try {
        const event = JSON.parse(data) as Record<string, unknown>;
        if (event.turn_id !== undefined && event.turn_id !== this.turnId) {
          throw new Error('turn mismatch');
        }
        this.callbacks.onError({
          code: typeof event.code === 'string' ? event.code : 'stream_error',
          message: typeof event.message === 'string' ? event.message : 'Response generation failed',
        });
      } catch {
        this.callbacks.onError({ code: 'stream_error', message: data });
      }
    }
  }

  private parseObject(data: string, code: string): Record<string, unknown> {
    try {
      const value = JSON.parse(data) as unknown;
      if (typeof value !== 'object' || value === null || Array.isArray(value)) throw new Error();
      return value as Record<string, unknown>;
    } catch {
      throw new ChatProtocolError(code, 'The response stream was malformed');
    }
  }
}

const MAX_CHAT_RETRIES = 3;
const RETRY_BASE_MS = 1_000;

function responseErrorBody(value: unknown, status: number): ChatStreamError {
  if (typeof value === 'object' && value !== null) {
    const detail = (value as { error?: unknown }).error;
    if (typeof detail === 'object' && detail !== null) {
      const code = (detail as { code?: unknown }).code;
      const message = (detail as { message?: unknown }).message;
      return {
        code: typeof code === 'string' ? code : `http_${status}`,
        message: typeof message === 'string' ? message : `HTTP ${status}`,
      };
    }
  }
  return { code: `http_${status}`, message: `HTTP ${status}` };
}

/** POST-based SSE chat with one idempotency key for every retry. */
export function createChatStream(options: ChatStreamOptions): () => void {
  let token = localStorage.getItem('auth_token');
  let sessionRefreshAttempted = false;
  let cancelled = false;
  let settled = false;
  let retries = 0;
  let retryTimer: ReturnType<typeof setTimeout> | undefined;
  let controller: AbortController | undefined;

  const fail = (error: ChatStreamError) => {
    if (cancelled || settled) return;
    settled = true;
    options.onError(error);
  };

  const scheduleRetry = (error: ChatStreamError, retryAfter: string | null = null) => {
    if (cancelled || settled) return;
    if (retries >= MAX_CHAT_RETRIES) {
      fail(error);
      return;
    }
    const exponential = RETRY_BASE_MS * 2 ** retries;
    const retryAfterMs = retryAfter && Number.isFinite(Number(retryAfter))
      ? Number(retryAfter) * 1_000
      : 0;
    retries++;
    options.onRetry?.();
    retryTimer = setTimeout(attempt, Math.max(exponential, retryAfterMs));
  };

  const consume = async (response: Response) => {
    const reader = response.body?.getReader();
    if (!reader) {
      throw new ChatProtocolError('missing_response_body', 'The response body is missing');
    }
    const decoder = new TextDecoder('utf-8', { fatal: true });
    const parser = new ChatSseParser(options.turnId, {
      onChunk: options.onChunk,
      onDone: result => {
        if (cancelled || settled) return;
        settled = true;
        options.onDone(result);
      },
      onError: fail,
    });

    while (!parser.isTerminal) {
      const { done, value } = await reader.read();
      if (done) {
        try {
          parser.feed(decoder.decode());
        } catch (error) {
          if (error instanceof ChatProtocolError) throw error;
          throw new ChatProtocolError('malformed_utf8', 'The response was not valid UTF-8');
        }
        parser.finish();
        break;
      }
      try {
        parser.feed(decoder.decode(value, { stream: true }));
      } catch (error) {
        if (error instanceof ChatProtocolError) throw error;
        throw new ChatProtocolError('malformed_utf8', 'The response was not valid UTF-8');
      }
    }

    if (parser.isTerminal) await reader.cancel().catch(() => undefined);
  };

  async function attempt() {
    if (cancelled || settled) return;
    controller = new AbortController();
    let response: Response;
    try {
      response = await fetch(`${BASE_URL}/chat/${options.characterId}/stream`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Idempotency-Key': options.turnId,
          ...(token ? { Authorization: `Bearer ${token}` } : {}),
        },
        body: JSON.stringify(options.payload),
        signal: controller.signal,
      });
    } catch (error) {
      if (cancelled || (error instanceof DOMException && error.name === 'AbortError')) return;
      scheduleRetry({
        code: 'network_error',
        message: error instanceof Error ? error.message : 'Network request failed',
      });
      return;
    }

    if (!response.ok) {
      if (response.status === 401 && token && !sessionRefreshAttempted) {
        sessionRefreshAttempted = true;
        try {
          token = await refreshSessionForAccessToken(token);
          await response.body?.cancel().catch(() => undefined);
          options.onRetry?.();
          await attempt();
          return;
        } catch (refreshError) {
          if (
            !(refreshError instanceof StaleSessionError)
            && !isDefinitiveRefreshRejection(refreshError)
          ) {
            await response.body?.cancel().catch(() => undefined);
            fail({
              code: 'session_refresh_unavailable',
              message: getApiErrorMessage(
                refreshError,
                'Session refresh is temporarily unavailable',
              ),
            });
            return;
          }
          // Fall through to the same session-fenced invalidation used by the
          // Axios interceptor. A newer principal is never cleared here.
        }
      }
      if (invalidateSessionForUnauthorizedRequest(
        response.status,
        token ? `Bearer ${token}` : undefined,
        `/chat/${options.characterId}/stream`,
      )) {
        reloadOrRedirectAfterUnauthorized();
      }
      const body = await response.json().catch(() => undefined);
      const error = responseErrorBody(body, response.status);
      if (response.status >= 500 || (response.status === 409 && error.code === 'turn_in_progress')) {
        scheduleRetry(error, response.headers.get('Retry-After'));
      } else {
        fail(error);
      }
      return;
    }

    try {
      await consume(response);
    } catch (error) {
      if (cancelled || (error instanceof DOMException && error.name === 'AbortError')) return;
      if (error instanceof ChatProtocolError) {
        fail({ code: error.code, message: error.message });
      } else {
        scheduleRetry({
          code: 'network_error',
          message: error instanceof Error ? error.message : 'The response stream failed',
        });
      }
    }
  }

  void attempt();
  return () => {
    cancelled = true;
    if (retryTimer !== undefined) clearTimeout(retryTimer);
    controller?.abort();
  };
}
