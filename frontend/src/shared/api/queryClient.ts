import { QueryClient } from '@tanstack/react-query';

export const PUBLIC_QUERY_SCOPE = 'public';

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      staleTime: 30_000,
    },
  },
});

export function clearPrivateQueryCache() {
  const isPrivate = (query: { queryKey: readonly unknown[] }) => (
    query.queryKey[0] !== PUBLIC_QUERY_SCOPE
  );
  void queryClient.cancelQueries({ predicate: isPrivate });
  queryClient.removeQueries({ predicate: isPrivate });
  queryClient.getMutationCache().clear();
}
