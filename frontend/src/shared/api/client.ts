import axios, { AxiosInstance, AxiosRequestConfig } from 'axios';

const BASE_URL = import.meta.env.VITE_API_URL || 'http://localhost:8080/api';

export const apiClient: AxiosInstance = axios.create({
  baseURL: BASE_URL,
  timeout: 30000,
  headers: { 'Content-Type': 'application/json' },
});

// 请求拦截器：注入 JWT
apiClient.interceptors.request.use((config) => {
  const token = localStorage.getItem('auth_token');
  if (token) {
    config.headers.Authorization = `Bearer ${token}`;
  }
  return config;
});

// 响应拦截器：统一错误处理
apiClient.interceptors.response.use(
  (response) => response,
  (error) => {
    if (error.response?.status === 401) {
      localStorage.removeItem('auth_token');
      window.location.href = '/login';
    }
    return Promise.reject(error);
  }
);

/** SSE 流式对话 */
export function createChatStream(
  characterId: string,
  payload: {
    user_id: string;
    novel_id: string;
    message: string;
    reader_identity?: string;
    current_chapter: number;
  },
  onChunk: (text: string) => void,
  onDone: () => void,
  onError: (err: string) => void,
): () => void {
  const token = localStorage.getItem('auth_token');
  const controller = new AbortController();
  let retryCount = 0;
  const maxRetries = 3;
  const baseDelay = 1000;

  const attempt = () => {
    fetch(`${BASE_URL}/chat/${characterId}/stream`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        ...(token ? { Authorization: `Bearer ${token}` } : {}),
      },
      body: JSON.stringify(payload),
      signal: controller.signal,
    }).then(async (res) => {
      if (!res.ok) {
        if (res.status >= 500 && retryCount < maxRetries) {
          retryCount++;
          setTimeout(attempt, baseDelay * Math.pow(2, retryCount - 1));
          return;
        }
        onError(`HTTP ${res.status}`);
        return;
      }
      retryCount = 0; // reset on success
      const reader = res.body?.getReader();
      if (!reader) { onError('No response body'); return; }

      const decoder = new TextDecoder();
      while (true) {
        const { done, value } = await reader.read();
        if (done) { onDone(); break; }
        const text = decoder.decode(value);
        const lines = text.split('\n');
        for (const line of lines) {
          if (line.startsWith('data: ')) {
            const data = line.slice(6);
            if (data.trim()) onChunk(data);
          } else if (line.startsWith('event: done')) {
            onDone();
            return;
          } else if (line.startsWith('event: error')) {
            onError('Stream error');
            return;
          }
        }
      }
    }).catch((err) => {
      if (err.name === 'AbortError') return;
      if (retryCount < maxRetries) {
        retryCount++;
        setTimeout(attempt, baseDelay * Math.pow(2, retryCount - 1));
        return;
      }
      onError(err.message);
    });
  };

  attempt();
  // 返回取消函数
  return () => controller.abort();
}
