import { lazy, Suspense, useEffect, useLayoutEffect, useRef } from 'react';
import { BrowserRouter, HashRouter, Routes, Route, Navigate } from 'react-router-dom';
import { QueryClientProvider } from '@tanstack/react-query';
import { Toaster } from 'sonner';
import { MotionConfig } from 'framer-motion';
import './styles/globals.css';

import { useSetupStatus } from '@/entities/runtime-config';
import { useAuthStore } from '@/features/auth';
import { useChatStore } from '@/features/character-chat';
import { clearPrivateQueryCache, queryClient } from '@/shared/api/queryClient';
import { isDesktopClient } from '@/shared/config/runtime';
import { useReducedMotionPreference } from '@/shared/lib/reducedMotion';

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
  const { user, fetchMe, logout, authStatus } = useAuthStore();
  const previousPrincipal = useRef<string | null>(null);
  const setupStatus = useSetupStatus();

  useLayoutEffect(() => {
    previousPrincipal.current = resetPrivateClientStateForPrincipalChange(
      previousPrincipal.current,
      user?.id ?? null,
    );
  }, [user?.id]);

  useEffect(() => {
    if (setupStatus.data?.configured) {
      void fetchMe();
    }
  }, [setupStatus.data?.configured, fetchMe]);

  const awaitingStoredSession = authStatus === 'idle'
    || authStatus === 'checking'
    || (authStatus === 'anonymous' && localStorage.getItem('auth_token') !== null);
  if (setupStatus.isPending || (setupStatus.data?.configured && awaitingStoredSession)) {
    return <AppLoadingScreen />;
  }

  if (setupStatus.isError) {
    return (
      <div className="app-surface flex min-h-screen items-center justify-center px-4">
        <div role="alert" className="surface-card max-w-md p-8 text-center text-[#5f6368]">
          <h1 className="mb-2 text-lg font-semibold text-[#1f1f1f]">
            无法检查服务配置
          </h1>
          <p className="mb-5 text-sm leading-6">NovelWorld 暂时无法连接到配置服务，请检查服务状态后重试。</p>
          <button
            onClick={() => { void setupStatus.refetch(); }}
            className="primary-action"
          >
            重试
          </button>
        </div>
      </div>
    );
  }

  if (setupStatus.data && !setupStatus.data.configured) {
    return (
      <SetupPage
        onComplete={() => { void setupStatus.refetch(); }}
      />
    );
  }

  if (authStatus === 'error') {
    return (
      <div className="app-surface flex min-h-screen items-center justify-center px-4">
        <div role="alert" className="surface-card max-w-md p-8 text-center text-[#5f6368]">
          <h1 className="mb-2 text-lg font-semibold text-[#1f1f1f]">
            暂时无法确认登录状态
          </h1>
          <p className="mb-5 text-sm leading-6">
            会话信息已保留。请检查网络或服务状态后重试。
          </p>
          <div className="flex justify-center gap-3">
            <button
              type="button"
              onClick={() => { void logout(); }}
              className="tonal-action"
            >
              退出登录
            </button>
            <button
              type="button"
              onClick={() => { void fetchMe(); }}
              className="primary-action"
            >
              重试
            </button>
          </div>
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
  const reducedMotion = useReducedMotionPreference();
  const Router = isDesktopClient ? HashRouter : BrowserRouter;
  useEffect(() => {
    const handleStorage = (event: StorageEvent) => {
      handleAuthTokenStorageChange(event);
    };
    window.addEventListener('storage', handleStorage);
    return () => window.removeEventListener('storage', handleStorage);
  }, []);
  return (
    <MotionConfig reducedMotion="user" skipAnimations={Boolean(reducedMotion)}>
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
    </MotionConfig>
  );
}
