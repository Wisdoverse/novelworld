import React, { useEffect } from 'react';
import { BrowserRouter, Routes, Route, Navigate } from 'react-router-dom';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { Toaster } from 'sonner';
import './styles/globals.css';

import { HomePage } from '@/pages/home/ui/HomePage';
import { LoginPage } from '@/pages/login/ui/LoginPage';
import { ShelfPage } from '@/pages/shelf/ui/ShelfPage';
import { ReaderPage } from '@/pages/reader/ui/ReaderPage';
import { CharactersPage } from '@/pages/characters/ui/CharactersPage';
import { useAuthStore } from '@/features/auth/model/useAuthStore';

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

  useEffect(() => {
    fetchMe();
  }, [fetchMe]);

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
