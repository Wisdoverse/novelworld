import { QueryClient } from '@tanstack/react-query';

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      staleTime: 30_000,
    },
  },
});

export function clearPrivateQueryCache() {
  void queryClient.cancelQueries();
  queryClient.clear();
}
