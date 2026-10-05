import { translate as t, useLocale } from '@/shared/lib/i18n';
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
import { PROVIDERS, providerOptions, type EditableProvider } from '../model/providers';
import { LanguageSwitcher } from '@/shared/ui/LanguageSwitcher';

type LlmSettings = {
  scope: LlmUsageScope;
  provider: EditableProvider | 'environment';
  model: string;
  thinking_enabled: boolean;
  api_key_configured: boolean;
};

const MODELS = {
  deepseek: [
    { id: 'deepseek-flash', label: 'DeepSeek V4.1 Flash', get hint() { return t("Text and images · Speed and cost first"); } },
    { id: 'deepseek-v4-pro', label: 'DeepSeek V4 Pro', get hint() { return t("Reasoning and complex tasks"); } },
  ],
  openai: [
    { id: 'gpt-4o-mini', label: 'GPT-4o mini', get hint() { return t("General lightweight model"); } },
  ],
} as const;

const LEGACY_DEEPSEEK_MODELS = new Set([
  'deepseek-v4-flash',
  'deepseek-v4-flash-vision-exp',
]);

export function SettingsPage() {
  useLocale();
  const navigate = useNavigate();
  const user = useAuthStore(state => state.user);
  const deleteAccount = useAuthStore(state => state.deleteAccount);
  const logout = useAuthStore(state => state.logout);
  const [settings, setSettings] = useState<LlmSettings | null>(null);
  const [settingsLoading, setSettingsLoading] = useState(true);
  const [settingsError, setSettingsError] = useState(false);
  const [apiKey, setApiKey] = useState('');
  const [savedProvider, setSavedProvider] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const isAdmin = user?.role === 'admin';
  const preset = settings && settings.provider !== 'environment'
    ? providerOptions.find(([id]) => id === settings.provider)?.[1]
    : undefined;
  const fixedModels = settings?.provider === 'deepseek' || settings?.provider === 'openai';
  const models = fixedModels ? MODELS[settings.provider as keyof typeof MODELS] : [];
  const keyRequired = !!settings && (!settings.api_key_configured || settings.provider !== savedProvider);
  const isEnvironmentManaged = isAdmin && settings?.provider === 'environment';
  const legacyDeepSeekModel = settings?.provider === 'deepseek'
    && LEGACY_DEEPSEEK_MODELS.has(settings.model)
    ? settings.model
    : null;

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
      setSavedProvider(response.data.provider);
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
      toast.error(getApiErrorMessage(error, t("Model settings failed to load")));
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
    if (provider === settings?.provider) return;
    setApiKey('');
    setSettings(current => current ? {
      ...current,
      provider,
      model: PROVIDERS[provider].models[0] ?? '',
      thinking_enabled: provider === 'deepseek' ? current.thinking_enabled : false,
    } : current);
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const principalId = user?.id;
    if (!settings || !principalId || settings.provider === 'environment') return;
    const trimmedApiKey = apiKey.trim();
    if (keyRequired && !trimmedApiKey) {
      toast.error(t("Enter a {p0} API key", { p0: isAdmin ? t("platform") : t("personal") }));
      return;
    }
    setSaving(true);
    try {
      const response = await apiClient.put<LlmSettings>('/settings/llm', {
        provider: settings.provider,
        model: legacyDeepSeekModel ? MODELS.deepseek[0].id : settings.model,
        thinking_enabled: settings.thinking_enabled,
        api_key: trimmedApiKey || undefined,
      });
      if (useAuthStore.getState().user?.id !== principalId) return;
      setSettings(response.data);
      setSavedProvider(response.data.provider);
      setApiKey('');
      void queryClient.invalidateQueries({
        queryKey: llmUsageKeys.summary(principalId, response.data.scope),
        exact: true,
      });
      toast.success(t("{p0} model settings saved. Future requests use them automatically.", { p0: isAdmin ? t("platform") : t("personal") }));
    } catch (error) {
      if (useAuthStore.getState().user?.id === principalId) {
        toast.error(getApiErrorMessage(error, t("Model settings failed to save")));
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
    if (!window.confirm(t("Permanently delete your account, shelf associations, reading progress, identities, chats, memories and personal timelines? Accepted source content remains with the shared original, including content still parsing or later failing. Removing a book or deleting the account does not delete that content. This cannot be undone."))) return;
    setDeleting(true);
    try {
      if (!await deleteAccount()) return;
      toast.success(t("Account and personal data deleted"));
      navigate('/', { replace: true });
    } catch (error) {
      toast.error(getApiErrorMessage(error, t("Account deletion failed. Try again later.")));
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
      toast.success(t("Account data exported"));
    } catch (error) {
      toast.error(getApiErrorMessage(error, t("Account export is incomplete. Try again.")));
    } finally {
      setExporting(false);
    }
  };

  return (
    <main className="app-surface min-h-screen px-4 py-8 sm:px-6 sm:py-10">
      <div className="mx-auto max-w-3xl">
        <button type="button" onClick={() => navigate('/shelf')} className="mb-6 flex items-center gap-2 text-sm font-medium text-[#0b57d0] hover:underline">
          <ArrowLeft size={16} /> {t("Back to shelf")}
        </button>

        <header className="mb-8">
          <div className="flex items-center justify-between gap-3">
            <div>
              <p className="text-sm font-medium text-[#0b57d0]">{t("Preferences and account")}</p>
              <h1 className="mt-2 text-3xl font-medium tracking-[-0.02em] text-[#1f1f1f]">{t("Settings")}</h1>
            </div>
            <LanguageSwitcher />
          </div>
          <p className="mt-2 text-sm text-[#5f6368]">{t("Manage models, API keys and account data.")}</p>
        </header>

        {settingsLoading && <div className="surface-card flex items-center justify-center p-10">
          <Loader2 className="animate-spin text-[#0b57d0]" aria-label={t("Loading model settings")} />
        </div>}

        {!settingsLoading && settingsError && <section className="surface-card p-6 sm:p-8" aria-labelledby="model-settings-heading">
          <h2 id="model-settings-heading" className="text-xl font-semibold text-[#1f1f1f]">{t("Model settings are temporarily unavailable")}</h2>
          <p className="mt-2 text-sm text-[#5f6368]">{t("Account data management remains available. Retry model settings later.")}</p>
          <button type="button" onClick={() => void loadSettings()} className="tonal-action mt-5">
            {t("Retry")}
          </button>
        </section>}

        {!settingsLoading && settings && <section className="surface-card p-6 sm:p-8" aria-labelledby="model-settings-heading">
          <div className="mb-7 flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-[#e8f0fe] text-[#0b57d0]">
              <Settings size={20} />
            </div>
            <div>
              <h2 id="model-settings-heading" className="text-xl font-semibold text-[#1f1f1f]">
                {isAdmin ? t("Platform model settings") : t("Personal model settings")}
              </h2>
              <p className="text-sm text-[#5f6368]">
                {isAdmin
                  ? t("Used for requests without a personal key")
                  : t("Used only for your requests. The platform key is neither shown nor changed.")}
              </p>
            </div>
          </div>

          {isEnvironmentManaged ? (
            <div className="rounded-2xl border border-[#a8c7fa] bg-[#eef4ff] p-4 text-sm text-[#3c4043]">
              <p className="font-semibold text-[#1f1f1f]">{t("The platform model is managed through environment variables")}</p>
              <p className="mt-1 leading-6">{t("Current model:")}{settings.model}{t(". Update platform configuration in the deployment environment.")}</p>
            </div>
          ) : <form onSubmit={submit} className="space-y-6">
            <label className="block text-sm font-medium text-[#3c4043]">
              {t("Provider / region / plan")}
              <select value={settings.provider} onChange={event => selectProvider(event.target.value as EditableProvider)} className="field-control mt-2">
                <optgroup label={t("Standard API")}>
                  {providerOptions.filter(([, provider]) => !provider.plan).map(([id, provider]) => <option key={id} value={id}>{provider.label}</option>)}
                </optgroup>
                <optgroup label="Coding Plan / Token Plan">
                  {providerOptions.filter(([, provider]) => provider.plan).map(([id, provider]) => <option key={id} value={id}>{provider.label}</option>)}
                </optgroup>
              </select>
            </label>
            {preset && <div className="text-xs leading-5 text-[#5f6368]">
              <p className="font-medium">{t("Current selection:")}{preset.label}</p>
              <p className="break-all">{t("Endpoint:")}{preset.endpoint}</p>
              <p>{t("The API key must match the selected region and plan. Enter a new key after switching.")}</p>
              {preset.plan && <p className="mt-2">{t("Consult your account console for quotas and supported models. The app will not switch to standard API; extra usage billing follows your account settings.")}</p>}
              {preset.restricted && <p className="mt-2">{t("This plan officially supports specific coding tools. NovelWorld's backend is not verified as supported, and a personal plan cannot be shared as a platform key. Use standard API or obtain provider authorization for this use.")}</p>}
            </div>}

            <label className="block text-sm font-medium text-[#3c4043]">
              {t("Model")}
              {fixedModels ? <select value={settings.model} onChange={event => setSettings({ ...settings, model: event.target.value })} className="field-control mt-2">
                {legacyDeepSeekModel && (
                  <option value={legacyDeepSeekModel} disabled>
                    {legacyDeepSeekModel} {t("— Legacy ID; saving switches to DeepSeek V4.1 Flash")}
                  </option>
                )}
                {models.map(model => <option key={model.id} value={model.id}>{model.label} — {model.hint}</option>)}
              </select> : <input value={settings.model} onChange={event => setSettings({ ...settings, model: event.target.value })} list="llm-model-suggestions" maxLength={200} required placeholder={t("Enter a model ID or inference endpoint ID available to your account")} className="field-control mt-2" />}
              <datalist id="llm-model-suggestions">
                {preset?.models.map(model => <option key={model} value={model} />)}
              </datalist>
            </label>

            {settings.provider === 'deepseek' && (
              <label className="flex cursor-pointer items-start justify-between gap-4 rounded-2xl border border-[#a8c7fa] bg-[#eef4ff] p-4">
                <span className="flex gap-3">
                  <Brain size={20} className="shrink-0 text-[#0b57d0]" />
                  <span>
                    <span className="block text-sm font-semibold text-[#1f1f1f]">{t("Enable thinking mode for character chat")}</span>
                    <span className="mt-1 block text-xs leading-5 text-[#5f6368]">{t("Uses the DeepSeek Responses API for reasoning and output. Structured novel parsing continues without thinking mode.")}</span>
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
                    ? !keyRequired
                      ? t("Platform API key (leave blank to retain the existing key)")
                      : t("Platform API key")
                    : !keyRequired
                      ? t("Personal API key (leave blank to retain the existing key)")
                      : t("Personal API key")}
                </span>
              </label>
              <input
                id="llm-api-key"
                type="password"
                value={apiKey}
                onChange={event => setApiKey(event.target.value)}
                autoComplete="off"
                required={keyRequired}
                aria-describedby={keyRequired ? 'llm-api-key-help' : undefined}
                placeholder={keyRequired ? t("Enter the API key for the selected region and plan") : t("Configured")}
                className="field-control mt-2"
              />
              {keyRequired && (
                <p id="llm-api-key-help" className="mt-2 text-xs font-normal leading-5 text-[#5f6368]">
                  {isAdmin
                    ? t("This selection requires its matching API key. The connection is verified before saving.")
                    : t("After configuring a personal key, you can view its usage. Until then, requests use the platform model.")}
                </p>
              )}
            </div>

            <div className="flex justify-end">
              <button type="submit" disabled={saving} className="primary-action">
                {saving ? <Loader2 size={16} className="animate-spin" /> : <Save size={16} />}
                {saving ? t("Verifying and saving…") : t("Save {p0} settings", { p0: isAdmin ? t("platform") : t("personal") })}
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
              <h2 id="account-settings-heading" className="text-xl font-semibold text-[#1f1f1f]">{t("Account data")}</h2>
              <p className="text-sm text-[#5f6368]">{user?.email}</p>
            </div>
          </div>
          <p className="mb-6 text-sm leading-6 text-[#5f6368]">
            {t("Export your account data first if needed. Deleting the account permanently removes login details, shelf associations, progress, identities, chats, memories and personal timelines. Accepted source content remains with the shared original, including content still parsing or later failing. After successful parsing, others can add it from the shared library. Removing a book or deleting your account does not delete this shared content.")}
          </p>
          <div className="flex flex-col gap-3 sm:flex-row sm:justify-end">
            <button type="button" disabled={exporting || deleting} onClick={() => { void logout(); }} className="tonal-action">
              <LogOut size={16} />
              {t("Sign out")}
            </button>
            <button type="button" disabled={exporting || deleting} onClick={exportAccount} className="tonal-action">
              {exporting ? <Loader2 size={16} className="animate-spin" /> : <Download size={16} />}
              {exporting ? t("Exporting…") : t("Export account data")}
            </button>
            <button type="button" disabled={deleting || exporting} onClick={eraseAccount} className="inline-flex min-h-11 items-center justify-center gap-2 rounded-full border border-[#b3261e] px-5 font-semibold text-[#b3261e] hover:bg-[#fce8e6] disabled:opacity-50">
              {deleting ? <Loader2 size={16} className="animate-spin" /> : <Trash2 size={16} />}
              {deleting ? t("Deleting…") : t("Delete account")}
            </button>
          </div>
        </section>
      </div>
    </main>
  );
}
