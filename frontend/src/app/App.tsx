import React, { lazy, Suspense, useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { BrowserRouter, HashRouter, Routes, Route, Navigate } from 'react-router-dom';
import { QueryClientProvider } from '@tanstack/react-query';
import { Toaster } from 'sonner';
import './styles/globals.css';

import { useAuthStore } from '@/features/auth';
import { useChatStore } from '@/features/character-chat';
import { apiClient } from '@/shared/api/client';
import { clearPrivateQueryCache, queryClient } from '@/shared/api/queryClient';
import { isDesktopClient } from '@/shared/config/runtime';

const HomePage = lazy(() => import('@/pages/home'));
const LoginPage = lazy(() => import('@/pages/login'));
const ShelfPage = lazy(() => import('@/pages/shelf'));
const ReaderPage = lazy(() => import('@/pages/reader'));
const CharactersPage = lazy(() => import('@/pages/characters'));
const SetupPage = lazy(() => import('@/pages/setup'));
const SettingsPage = lazy(() => import('@/pages/settings'));

function AppLoadingScreen() {
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

export function resetPrivateClientStateForPrincipalChange(
  previousPrincipal: string | null,
  currentPrincipal: string | null,
) {
  if (previousPrincipal !== currentPrincipal) {
    useChatStore.getState().reset();
  }
  return currentPrincipal;
}

export function handleAuthTokenStorageChange(
  event: Pick<StorageEvent, 'key' | 'oldValue' | 'newValue'>,
  reload: () => void = () => window.location.reload(),
) {
  if (event.key !== 'auth_token' || event.oldValue === event.newValue) return false;
  clearPrivateQueryCache();
  useChatStore.getState().reset();
  reload();
  return true;
}

function AppRouteContent() {
  const { user, fetchMe } = useAuthStore();
  const previousPrincipal = useRef<string | null>(null);
  const [setupStatus, setSetupStatus] = useState<'loading' | 'needed' | 'done' | 'error'>('loading');
  const [llmConfigured, setLlmConfigured] = useState(false);
  const [authReady, setAuthReady] = useState(false);

  useLayoutEffect(() => {
    previousPrincipal.current = resetPrivateClientStateForPrincipalChange(
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
    return <AppLoadingScreen />;
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

export function AppRoutes() {
  return (
    <Suspense fallback={<AppLoadingScreen />}>
      <AppRouteContent />
    </Suspense>
  );
}

export function App() {
  const Router = isDesktopClient ? HashRouter : BrowserRouter;
  useEffect(() => {
    const handleStorage = (event: StorageEvent) => {
      handleAuthTokenStorageChange(event);
    };
    window.addEventListener('storage', handleStorage);
    return () => window.removeEventListener('storage', handleStorage);
  }, []);
  return (
    <QueryClientProvider client={queryClient}>
      <Router>
        <AppRoutes />
      </Router>
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
