import React, { useEffect, useState } from 'react';
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
import { useAuthStore } from '@/features/auth/model/useAuthStore';
import { apiClient } from '@/shared/api/client';

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      staleTime: 30_000,
    },
  },
});

function AppRoutes() {
  const { user, fetchMe } = useAuthStore();
  const [setupStatus, setSetupStatus] = useState<'loading' | 'needed' | 'done'>('loading');

  useEffect(() => {
    apiClient.get('/setup/status')
      .then(res => {
        setSetupStatus(res.data?.configured ? 'done' : 'needed');
      })
      .catch(() => {
        setSetupStatus('done');
      });
  }, []);

  useEffect(() => {
    if (setupStatus === 'done') {
      fetchMe();
    }
  }, [setupStatus, fetchMe]);

  if (setupStatus === 'loading') {
    return (
      <div className="min-h-screen flex items-center justify-center"
           style={{ background: 'linear-gradient(135deg, var(--color-void) 0%, var(--color-cosmos) 100%)' }}>
        <div className="text-center">
          <div className="w-8 h-8 border-2 border-t-transparent rounded-full animate-spin mx-auto mb-4"
               style={{ borderColor: 'var(--color-nova-glow)', borderTopColor: 'transparent' }} />
          <p style={{ color: 'var(--color-moonbeam)' }}>Loading...</p>
        </div>
      </div>
    );
  }

  if (setupStatus === 'needed') {
    return <SetupPage onComplete={() => {
      setSetupStatus('done');
      fetchMe();
    }} />;
  }

  return (
    <Routes>
      <Route path="/" element={<HomePage />} />
      <Route path="/login" element={<LoginPage />} />
      <Route path="/shelf" element={user ? <ShelfPage /> : <Navigate to="/login" replace />} />
      <Route path="/reader/:novelId/:chapterNum" element={user ? <ReaderPage /> : <Navigate to="/login" replace />} />
      <Route path="/reader/:novelId" element={<Navigate to="1" replace />} />
      <Route path="/characters/:novelId" element={user ? <CharactersPage /> : <Navigate to="/login" replace />} />
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
            background: 'rgba(15, 21, 53, 0.95)',
            border: '1px solid rgba(109, 40, 217, 0.3)',
            color: '#e2e8f0',
          },
        }}
      />
    </QueryClientProvider>
  );
}
