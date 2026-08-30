import { useCallback, useEffect, useState, type FormEvent } from 'react';
import { ArrowLeft, Brain, Download, Key, Loader2, LogOut, Save, Settings, Trash2 } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { toast } from 'sonner';
import {
  llmUsageKeys,
  type LlmUsageScope,
} from '@/entities/llm-usage';
import { apiClient, getApiErrorCode, getApiErrorMessage } from '@/shared/api/client';
import { queryClient } from '@/shared/api/queryClient';
import { useAuthStore } from '@/features/auth';
import { LlmUsageCard } from '@/features/llm-usage';

type LlmSettings = {
  scope: LlmUsageScope;
  provider: EditableProvider | 'environment';
  model: string;
  thinking_enabled: boolean;
  api_key_configured: boolean;
};

const MODELS = {
  deepseek: [
    { id: 'deepseek-v4-flash', label: 'DeepSeek V4 Flash', hint: '速度与成本优先' },
    { id: 'deepseek-v4-flash-vision-exp', label: 'DeepSeek V4 Flash Vision', hint: '实验模型 · 兼容接口' },
    { id: 'deepseek-v4-pro', label: 'DeepSeek V4 Pro', hint: '质量与复杂推理优先' },
  ],
  openai: [
    { id: 'gpt-4o-mini', label: 'GPT-4o mini', hint: '通用轻量模型' },
  ],
} as const;

type EditableProvider = keyof typeof MODELS;

export function SettingsPage() {
  const navigate = useNavigate();
  const user = useAuthStore(state => state.user);
  const deleteAccount = useAuthStore(state => state.deleteAccount);
  const logout = useAuthStore(state => state.logout);
  const [settings, setSettings] = useState<LlmSettings | null>(null);
  const [settingsLoading, setSettingsLoading] = useState(true);
  const [settingsError, setSettingsError] = useState(false);
  const [apiKey, setApiKey] = useState('');
  const [saving, setSaving] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const isAdmin = user?.role === 'admin';
  const models = settings && settings.provider !== 'environment' ? MODELS[settings.provider] : [];
  const isEnvironmentManaged = isAdmin && settings?.provider === 'environment';

  const loadSettings = useCallback(async () => {
    const principalId = user?.id;
    if (!principalId) return;
    setSettings(null);
    setSettingsError(false);
    setSettingsLoading(true);
    try {
      const response = await apiClient.get<LlmSettings>('/settings/llm');
      const currentUser = useAuthStore.getState().user;
      if (currentUser?.id !== principalId) return;
      setSettings(currentUser.role !== 'admin' && response.data.provider === 'environment'
        ? {
            ...response.data,
            provider: 'deepseek',
            model: MODELS.deepseek[0].id,
            thinking_enabled: false,
          }
        : response.data);
    } catch (error) {
      if (useAuthStore.getState().user?.id !== principalId) return;
      if (isAdmin && getApiErrorCode(error) === 'setup_required') {
        setSettings({
          scope: 'platform',
          provider: 'deepseek',
          model: MODELS.deepseek[0].id,
          thinking_enabled: false,
          api_key_configured: false,
        });
        return;
      }
      setSettingsError(true);
      toast.error(getApiErrorMessage(error, '模型设置加载失败'));
    } finally {
      if (useAuthStore.getState().user?.id === principalId) setSettingsLoading(false);
    }
  }, [isAdmin, user?.id]);

  useEffect(() => {
    void loadSettings();
  }, [loadSettings]);

  useEffect(() => {
    setApiKey('');
  }, [user?.id]);

  const selectProvider = (provider: EditableProvider) => {
    setSettings(current => current ? {
      ...current,
      provider,
      model: MODELS[provider][0].id,
      thinking_enabled: provider === 'deepseek' ? current.thinking_enabled : false,
    } : current);
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const principalId = user?.id;
    if (!settings || !principalId || settings.provider === 'environment') return;
    const trimmedApiKey = apiKey.trim();
    if (!settings.api_key_configured && !trimmedApiKey) {
      toast.error(`请输入${isAdmin ? '平台' : '个人'} API Key`);
      return;
    }
    setSaving(true);
    try {
      const response = await apiClient.put<LlmSettings>('/settings/llm', {
        provider: settings.provider,
        model: settings.model,
        thinking_enabled: settings.thinking_enabled,
        api_key: trimmedApiKey || undefined,
      });
      if (useAuthStore.getState().user?.id !== principalId) return;
      setSettings(response.data);
      setApiKey('');
      void queryClient.invalidateQueries({
        queryKey: llmUsageKeys.summary(principalId, response.data.scope),
        exact: true,
      });
      toast.success(`${isAdmin ? '平台' : '个人'}模型设置已保存，后续请求自动生效`);
    } catch (error) {
      if (useAuthStore.getState().user?.id === principalId) {
        toast.error(getApiErrorMessage(error, '模型设置保存失败'));
      }
    } finally {
      setSaving(false);
    }
  };

  const usageScope: LlmUsageScope | null = isAdmin && settings?.scope === 'platform'
    ? 'platform'
    : !isAdmin && settings?.scope === 'user' && settings.api_key_configured
      ? 'user'
      : null;

  const eraseAccount = async () => {
    if (!window.confirm('永久删除你的账号、书架关联、阅读进度、身份、对话、记忆和个人时间线？你提交并被系统接受的来源内容，包括仍在解析或随后解析失败的内容，会随共享原著继续保留；移出书架或删除账号不会删除它们。此操作无法撤销。')) return;
    setDeleting(true);
    try {
      if (!await deleteAccount()) return;
      toast.success('账号和个人数据已删除');
      navigate('/', { replace: true });
    } catch (error) {
      toast.error(getApiErrorMessage(error, '账号删除失败，请稍后重试'));
    } finally {
      setDeleting(false);
    }
  };

  const exportAccount = async () => {
    const token = localStorage.getItem('auth_token');
    const userId = user?.id;
    const principalIsCurrent = () => (
      localStorage.getItem('auth_token') === token
      && useAuthStore.getState().user?.id === userId
    );
    setExporting(true);
    try {
      // ponytail: A browser Blob prevents saving partial exports; adopt the File
      // System Access API only if measured account sizes make this impractical.
      const response = await apiClient.get<Blob>('/account/export', {
        responseType: 'blob',
        timeout: 16 * 60 * 1000,
      });
      if (!principalIsCurrent()) return;
      const tail = await response.data.slice(Math.max(0, response.data.size - 4096)).text();
      if (!principalIsCurrent()) return;
      const lines = tail.trimEnd().split('\n');
      const lastLine = lines[lines.length - 1];
      const completion = lastLine ? JSON.parse(lastLine) : null;
      if (completion?.type !== 'complete' || completion?.schema !== 'account-export-v1') {
        throw new Error('incomplete account export');
      }

      const url = URL.createObjectURL(response.data);
      const link = document.createElement('a');
      link.href = url;
      link.download = `novelworld-account-${userId}-${new Date().toISOString().slice(0, 10)}.ndjson`;
      document.body.appendChild(link);
      link.click();
      link.remove();
      URL.revokeObjectURL(url);
      toast.success('账号数据导出完成');
    } catch (error) {
      toast.error(getApiErrorMessage(error, '账号导出未完整完成，请重试'));
    } finally {
      setExporting(false);
    }
  };

  return (
    <main className="app-surface min-h-screen px-4 py-8 sm:px-6 sm:py-10">
      <div className="mx-auto max-w-3xl">
        <button type="button" onClick={() => navigate('/shelf')} className="mb-6 flex items-center gap-2 text-sm font-medium text-[#0b57d0] hover:underline">
          <ArrowLeft size={16} /> 返回书架
        </button>

        <header className="mb-8">
          <p className="text-sm font-medium text-[#0b57d0]">偏好与账号</p>
          <h1 className="mt-2 text-3xl font-medium tracking-[-0.02em] text-[#1f1f1f]">设置</h1>
          <p className="mt-2 text-sm text-[#5f6368]">管理模型、API Key 与账号数据。</p>
        </header>

        {settingsLoading && <div className="surface-card flex items-center justify-center p-10">
          <Loader2 className="animate-spin text-[#0b57d0]" aria-label="正在加载模型设置" />
        </div>}

        {!settingsLoading && settingsError && <section className="surface-card p-6 sm:p-8" aria-labelledby="model-settings-heading">
          <h2 id="model-settings-heading" className="text-xl font-semibold text-[#1f1f1f]">模型设置暂时不可用</h2>
          <p className="mt-2 text-sm text-[#5f6368]">账号数据仍可正常管理，请稍后重试模型设置。</p>
          <button type="button" onClick={() => void loadSettings()} className="tonal-action mt-5">
            重试
          </button>
        </section>}

        {!settingsLoading && settings && <section className="surface-card p-6 sm:p-8" aria-labelledby="model-settings-heading">
          <div className="mb-7 flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-[#e8f0fe] text-[#0b57d0]">
              <Settings size={20} />
            </div>
            <div>
              <h2 id="model-settings-heading" className="text-xl font-semibold text-[#1f1f1f]">
                {isAdmin ? '平台模型设置' : '个人模型设置'}
              </h2>
              <p className="text-sm text-[#5f6368]">
                {isAdmin
                  ? '供未配置个人 Key 的请求使用'
                  : '仅你的请求使用，不会显示或修改平台 Key'}
              </p>
            </div>
          </div>

          {isEnvironmentManaged ? (
            <div className="rounded-2xl border border-[#a8c7fa] bg-[#eef4ff] p-4 text-sm text-[#3c4043]">
              <p className="font-semibold text-[#1f1f1f]">平台模型由环境变量管理</p>
              <p className="mt-1 leading-6">当前模型：{settings.model}。请在部署环境中更新平台配置。</p>
            </div>
          ) : <form onSubmit={submit} className="space-y-6">
            <fieldset>
              <legend className="mb-2 text-sm font-medium text-[#3c4043]">服务商</legend>
              <div className="grid grid-cols-2 gap-3">
                {(['deepseek', 'openai'] as const).map(provider => {
                  const selected = settings.provider === provider;
                  return (
                    <button key={provider} type="button" aria-pressed={selected} onClick={() => selectProvider(provider)} className={`rounded-xl border px-4 py-3 text-left text-sm font-semibold transition-colors ${selected ? 'border-[#0b57d0] bg-[#e8f0fe] text-[#174ea6]' : 'border-[#dadce0] bg-white text-[#3c4043] hover:bg-[#f8fafd]'}`}>
                      {provider === 'deepseek' ? 'DeepSeek' : 'OpenAI'}
                    </button>
                  );
                })}
              </div>
            </fieldset>

            <label className="block text-sm font-medium text-[#3c4043]">
              模型
              <select value={settings.model} onChange={event => setSettings({ ...settings, model: event.target.value })} className="field-control mt-2">
                {models.map(model => <option key={model.id} value={model.id}>{model.label} — {model.hint}</option>)}
              </select>
            </label>

            {settings.provider === 'deepseek' && (
              <label className="flex cursor-pointer items-start justify-between gap-4 rounded-2xl border border-[#a8c7fa] bg-[#eef4ff] p-4">
                <span className="flex gap-3">
                  <Brain size={20} className="shrink-0 text-[#0b57d0]" />
                  <span>
                    <span className="block text-sm font-semibold text-[#1f1f1f]">角色对话启用思考模式</span>
                    <span className="mt-1 block text-xs leading-5 text-[#5f6368]">通过 DeepSeek Responses API 处理推理与输出；小说结构化解析继续使用非思考模式。</span>
                  </span>
                </span>
                <input type="checkbox" checked={settings.thinking_enabled} onChange={event => setSettings({ ...settings, thinking_enabled: event.target.checked })} className="mt-1 h-5 w-5 accent-[#0b57d0]" />
              </label>
            )}

            <div>
              <label htmlFor="llm-api-key" className="block text-sm font-medium text-[#3c4043]">
                <span className="flex items-center gap-2">
                  <Key size={15} />
                  {isAdmin
                    ? settings.api_key_configured
                      ? '平台 API Key（留空则保持现有 Key）'
                      : '平台 API Key'
                    : settings.api_key_configured
                      ? '个人 API Key（留空则保持现有 Key）'
                      : '个人 API Key'}
                </span>
              </label>
              <input
                id="llm-api-key"
                type="password"
                value={apiKey}
                onChange={event => setApiKey(event.target.value)}
                autoComplete="off"
                required={!settings.api_key_configured}
                aria-describedby={!settings.api_key_configured ? 'llm-api-key-help' : undefined}
                placeholder={settings.api_key_configured ? '已配置' : '请输入 API Key'}
                className="field-control mt-2"
              />
              {!settings.api_key_configured && (
                <p id="llm-api-key-help" className="mt-2 text-xs font-normal leading-5 text-[#5f6368]">
                  {isAdmin
                    ? '首次配置平台模型需要 API Key；保存前会验证连接。'
                    : '配置个人 Key 后，可查看该 Key 的消耗；配置前继续使用平台模型。'}
                </p>
              )}
            </div>

            <div className="flex justify-end">
              <button type="submit" disabled={saving} className="primary-action">
                {saving ? <Loader2 size={16} className="animate-spin" /> : <Save size={16} />}
                {saving ? '正在验证并保存…' : `保存${isAdmin ? '平台' : '个人'}设置`}
              </button>
            </div>
          </form>}
        </section>}

        {user && usageScope && (
          <LlmUsageCard principalId={user.id} scope={usageScope} />
        )}

        <section className="surface-card mt-6 p-6 sm:p-8" aria-labelledby="account-settings-heading">
          <div className="mb-5 flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-[#fce8e6] text-[#b3261e]">
              <Trash2 size={20} />
            </div>
            <div>
              <h2 id="account-settings-heading" className="text-xl font-semibold text-[#1f1f1f]">账号数据</h2>
              <p className="text-sm text-[#5f6368]">{user?.email}</p>
            </div>
          </div>
          <p className="mb-6 text-sm leading-6 text-[#5f6368]">
            你可以先导出账号数据。删除账号会永久删除你的登录资料、书架关联、阅读进度、身份、对话、记忆和个人时间线；你提交并被系统接受的来源内容，包括仍在解析或随后解析失败的内容，会随共享原著继续保留。解析成功后其他用户仍可从共享书库加入，移出书架或删除账号不会删除这些共享内容。
          </p>
          <div className="flex flex-col gap-3 sm:flex-row sm:justify-end">
            <button type="button" disabled={exporting || deleting} onClick={() => { void logout(); }} className="tonal-action">
              <LogOut size={16} />
              退出登录
            </button>
            <button type="button" disabled={exporting || deleting} onClick={exportAccount} className="tonal-action">
              {exporting ? <Loader2 size={16} className="animate-spin" /> : <Download size={16} />}
              {exporting ? '正在导出…' : '导出账号数据'}
            </button>
            <button type="button" disabled={deleting || exporting} onClick={eraseAccount} className="inline-flex min-h-11 items-center justify-center gap-2 rounded-full border border-[#b3261e] px-5 font-semibold text-[#b3261e] hover:bg-[#fce8e6] disabled:opacity-50">
              {deleting ? <Loader2 size={16} className="animate-spin" /> : <Trash2 size={16} />}
              {deleting ? '正在删除…' : '删除账号'}
            </button>
          </div>
        </section>
      </div>
    </main>
  );
}
