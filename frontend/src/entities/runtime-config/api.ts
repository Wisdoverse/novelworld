import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { apiClient } from '@/shared/api/client';
import { PUBLIC_QUERY_SCOPE } from '@/shared/api/queryClient';

export type SetupStatus = {
  contract: 4;
  configured: boolean;
  admin_configured: boolean;
  llm_configured: boolean;
};

export type LlmSettings = {
  provider: string;
  model: string;
  thinking_enabled: boolean;
  api_key_configured: boolean;
};

export type UpdateLlmSettings = Pick<LlmSettings, 'provider' | 'model' | 'thinking_enabled'> & {
  api_key?: string;
};

export const runtimeConfigKeys = {
  setup: [PUBLIC_QUERY_SCOPE, 'setup-status'] as const,
  llm: (userId: string) => ['runtime-config', 'llm', userId] as const,
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

export function useLlmSettings(userId: string, enabled: boolean) {
  return useQuery({
    queryKey: runtimeConfigKeys.llm(userId),
    queryFn: () => apiClient.get<LlmSettings>('/settings/llm').then(response => response.data),
    enabled,
    retry: false,
    refetchOnWindowFocus: false,
  });
}

export function useUpdateLlmSettings(userId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (settings: UpdateLlmSettings) =>
      apiClient.put<LlmSettings>('/settings/llm', settings).then(response => response.data),
    onSuccess: settings => {
      queryClient.setQueryData(runtimeConfigKeys.llm(userId), settings);
      void queryClient.invalidateQueries({ queryKey: runtimeConfigKeys.setup });
    },
  });
}
