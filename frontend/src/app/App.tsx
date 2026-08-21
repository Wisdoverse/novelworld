import React, { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { Toaster } from 'sonner';
import './styles/globals.css';

import { HomePage } from '@/pages/home/ui/HomePage';
import { LoginPage } from '@/pages/login/ui/LoginPage';
import { ShelfPage } from '@/pages/shelf/ui/ShelfPage';
import { ReaderPage } from '@/pages/reader/ui/ReaderPage';
import { CharactersPage } from '@/pages/characters/ui/CharactersPage';
import { SetupPage } from '@/pages/setup/ui/SetupPage';
import { SettingsPage } from '@/pages/settings/ui/SettingsPage';
import { useAuthStore } from '@/features/auth/model/useAuthStore';
import { useChatStore } from '@/features/character-chat/model/useChatStore';
import { apiClient } from '@/shared/api/client';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      staleTime: 30_000,
    },
  },
});

export function resetPrivateClientStateForPrincipalChange(
  client: QueryClient,
  previousPrincipal: string | null,
  currentPrincipal: string | null,
) {
  if (previousPrincipal !== currentPrincipal) {
    client.clear();
    useChatStore.getState().reset();
  }
  return currentPrincipal;
}

export function AppRoutes() {
  const { user, fetchMe } = useAuthStore();
  const previousPrincipal = useRef<string | null>(null);
  const [setupStatus, setSetupStatus] = useState<'loading' | 'needed' | 'done' | 'error'>('loading');
  const [llmConfigured, setLlmConfigured] = useState(false);
  const [authReady, setAuthReady] = useState(false);

  useLayoutEffect(() => {
    previousPrincipal.current = resetPrivateClientStateForPrincipalChange(
      queryClient,
      previousPrincipal.current,
      user?.id ?? null,
    );
  }, [user?.id]);

  const loadSetupStatus = useCallback(() => {
    setSetupStatus('loading');
    apiClient.get('/setup/status')
      .then(res => {
        if (res.data?.contract !== 3) {
          setSetupStatus('error');
          return;
        }
        setLlmConfigured(res.data.llm_configured === true);
        setSetupStatus(res.data.configured ? 'done' : 'needed');
      })
      .catch(() => {
        setSetupStatus('error');
      });
  }, []);

  useEffect(() => {
    loadSetupStatus();
  }, [loadSetupStatus]);

  useEffect(() => {
    if (setupStatus === 'done') {
      let active = true;
      setAuthReady(false);
      fetchMe().finally(() => {
        if (active) setAuthReady(true);
      });
      return () => {
        active = false;
      };
    }
    setAuthReady(false);
  }, [setupStatus, fetchMe]);

  if (setupStatus === 'loading' || (setupStatus === 'done' && !authReady)) {
    return (
      <div className="app-surface flex min-h-screen items-center justify-center">
        <div className="text-center">
          <div className="w-8 h-8 border-2 border-t-transparent rounded-full animate-spin mx-auto mb-4"
               style={{ borderColor: '#0b57d0', borderTopColor: 'transparent' }} />
          <p className="text-sm text-[#5f6368]">正在加载…</p>
        </div>
      </div>
    );
  }

  if (setupStatus === 'needed') {
    return (
      <SetupPage
        llmConfigured={llmConfigured}
        onComplete={() => setSetupStatus('done')}
      />
    );
  }

  if (setupStatus === 'error') {
    return (
      <div className="app-surface flex min-h-screen items-center justify-center px-4">
        <div role="alert" className="surface-card max-w-md p-8 text-center text-[#5f6368]">
          <h1 className="mb-2 text-lg font-semibold text-[#1f1f1f]">
            无法检查服务配置
          </h1>
          <p className="mb-5 text-sm leading-6">NovelWorld 暂时无法连接到配置服务，请检查服务状态后重试。</p>
          <button
            onClick={loadSetupStatus}
            className="primary-action"
          >
            重试
          </button>
        </div>
      </div>
    );
  }

  return (
    <Routes>
      <Route path="/" element={<HomePage />} />
      <Route path="/login" element={<LoginPage />} />
      <Route path="/register" element={<LoginPage initialRegister />} />
      <Route path="/shelf" element={user ? <ShelfPage /> : <Navigate to="/login" replace />} />
      <Route path="/reader/:novelId/:chapterNum" element={user ? <ReaderPage /> : <Navigate to="/login" replace />} />
      <Route path="/reader/:novelId" element={user ? <ReaderPage /> : <Navigate to="/login" replace />} />
      <Route path="/characters/:novelId" element={user ? <CharactersPage /> : <Navigate to="/login" replace />} />
      <Route path="/settings" element={user ? <SettingsPage /> : <Navigate to="/login" replace />} />
      <Route path="*" element={<Navigate to="/" replace />} />
    </Routes>
  );
}

export function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <AppRoutes />
      </BrowserRouter>
      <Toaster
        position="bottom-right"
        toastOptions={{
          style: {
            background: '#fff',
            border: '1px solid #e1e3e8',
            color: '#1f1f1f',
            boxShadow: '0 8px 28px rgba(60,64,67,0.14)',
          },
        }}
      />
    </QueryClientProvider>
  );
}
