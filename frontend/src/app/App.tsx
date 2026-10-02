import { translate as t, useLocale } from '@/shared/lib/i18n';
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
import { LanguageSwitcher } from '@/shared/ui/LanguageSwitcher';

const HomePage = lazy(() => import('@/pages/home'));
const LoginPage = lazy(() => import('@/pages/login'));
const ShelfPage = lazy(() => import('@/pages/shelf'));
const ReaderPage = lazy(() => import('@/pages/reader'));
const CharactersPage = lazy(() => import('@/pages/characters'));
const SetupPage = lazy(() => import('@/pages/setup'));
const SettingsPage = lazy(() => import('@/pages/settings'));

function AppLoadingScreen() {
  useLocale();
  return (
    <div className="app-surface flex min-h-screen items-center justify-center">
      <div className="text-center">
        <div className="w-8 h-8 border-2 border-t-transparent rounded-full animate-spin mx-auto mb-4"
             style={{ borderColor: '#0b57d0', borderTopColor: 'transparent' }} />
        <p className="text-sm text-[#5f6368]">{t("Loading…")}</p>
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
  useLocale();
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
            {t("Cannot check service configuration")}
          </h1>
          <p className="mb-5 text-sm leading-6">{t("NovelWorld cannot reach the configuration service. Check the service status and try again.")}</p>
          <button
            onClick={() => { void setupStatus.refetch(); }}
            className="primary-action"
          >
            {t("Retry")}
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
            {t("Cannot verify your session right now")}
          </h1>
          <p className="mb-5 text-sm leading-6">
            {t("Your session has been retained. Check your connection or service status and try again.")}
          </p>
          <div className="flex justify-center gap-3">
            <button
              type="button"
              onClick={() => { void logout(); }}
              className="tonal-action"
            >
              {t("Sign out")}
            </button>
            <button
              type="button"
              onClick={() => { void fetchMe(); }}
              className="primary-action"
            >
              {t("Retry")}
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
  useLocale();
  return (
    <Suspense fallback={<AppLoadingScreen />}>
      <AppRouteContent />
    </Suspense>
  );
}

export function App() {
  const locale = useLocale();
  const reducedMotion = useReducedMotionPreference();
  const Router = isDesktopClient ? HashRouter : BrowserRouter;
  useLayoutEffect(() => {
    document.documentElement.lang = locale;
    document.title = t('NovelWorld — Enter a novel world');
    document.querySelector('meta[name="description"]')?.setAttribute(
      'content', t('Import novels, meet their characters and shape your own story.'),
    );
  }, [locale]);
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
      <div className="flex justify-end border-b border-[#e8eaed] bg-white px-4 py-2 sm:px-6">
        <LanguageSwitcher />
      </div>
      <Router>
        <AppRoutes />
      </Router>
      <Toaster
        containerAriaLabel={t('Notifications')}
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
