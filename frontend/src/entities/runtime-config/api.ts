import { useQuery } from '@tanstack/react-query';
import { apiClient } from '@/shared/api/client';
import { PUBLIC_QUERY_SCOPE } from '@/shared/api/queryClient';

export type SetupStatus = {
  contract: 4;
  configured: boolean;
  admin_configured: boolean;
  llm_configured: boolean;
};

export const runtimeConfigKeys = {
  setup: [PUBLIC_QUERY_SCOPE, 'setup-status'] as const,
};

export function useSetupStatus() {
  return useQuery({
    queryKey: runtimeConfigKeys.setup,
    queryFn: async () => {
      const response = await apiClient.get<SetupStatus>('/setup/status');
      if (response.data.contract !== 4) throw new Error('Unsupported setup contract');
      return response.data;
    },
    retry: false,
  });
}
