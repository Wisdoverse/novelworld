import { useEffect, useMemo, useState, type FormEvent } from 'react';
import { ArrowLeft, Brain, Key, Loader2, Save, Settings, Trash2 } from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { toast } from 'sonner';
import { apiClient, getApiErrorMessage } from '@/shared/api/client';
import { useAuthStore } from '@/features/auth/model/useAuthStore';

type LlmSettings = {
  provider: 'deepseek' | 'openai';
  model: string;
  thinking_enabled: boolean;
  api_key_configured: boolean;
};

const MODELS = {
  deepseek: [
    { id: 'deepseek-v4-flash', label: 'DeepSeek V4 Flash', hint: '速度与成本优先' },
    { id: 'deepseek-v4-pro', label: 'DeepSeek V4 Pro', hint: '质量与复杂推理优先' },
  ],
  openai: [
    { id: 'gpt-4o-mini', label: 'GPT-4o mini', hint: '通用轻量模型' },
  ],
} as const;

export function SettingsPage() {
  const navigate = useNavigate();
  const user = useAuthStore(state => state.user);
  const deleteAccount = useAuthStore(state => state.deleteAccount);
  const [settings, setSettings] = useState<LlmSettings | null>(null);
  const [apiKey, setApiKey] = useState('');
  const [saving, setSaving] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const isAdmin = user?.role === 'admin';
  const models = useMemo(() => settings ? MODELS[settings.provider] : [], [settings]);

  useEffect(() => {
    if (!isAdmin) return;
    apiClient.get<LlmSettings>('/settings/llm')
      .then(response => setSettings(response.data))
      .catch(error => toast.error(getApiErrorMessage(error, '模型设置加载失败')));
  }, [isAdmin]);

  const selectProvider = (provider: LlmSettings['provider']) => {
    setSettings(current => current ? {
      ...current,
      provider,
      model: MODELS[provider][0].id,
      thinking_enabled: provider === 'deepseek' ? current.thinking_enabled : false,
    } : current);
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!settings) return;
    setSaving(true);
    try {
      const response = await apiClient.put<LlmSettings>('/settings/llm', {
        provider: settings.provider,
        model: settings.model,
        thinking_enabled: settings.thinking_enabled,
        api_key: apiKey.trim() || undefined,
      });
      setSettings(response.data);
      setApiKey('');
      toast.success('模型设置已保存，后续请求自动生效');
    } catch (error) {
      toast.error(getApiErrorMessage(error, '模型设置保存失败'));
    } finally {
      setSaving(false);
    }
  };

  const eraseAccount = async () => {
    if (!window.confirm('永久删除账号及全部小说、对话、记忆和时间线？此操作无法撤销。')) return;
    setDeleting(true);
    try {
      await deleteAccount();
      toast.success('账号数据已删除');
      navigate('/', { replace: true });
    } catch (error) {
      toast.error(getApiErrorMessage(error, '账号删除失败，请稍后重试'));
    } finally {
      setDeleting(false);
    }
  };

  return (
    <main className="min-h-screen px-4 py-8" style={{ background: 'var(--color-void)' }}>
      <div className="mx-auto max-w-2xl">
        <button type="button" onClick={() => navigate('/shelf')} className="mb-6 flex items-center gap-2 text-sm" style={{ color: '#94a3b8' }}>
          <ArrowLeft size={16} /> 返回书架
        </button>
        {isAdmin && !settings && <div className="glass-card flex items-center justify-center p-8">
          <Loader2 className="animate-spin" style={{ color: '#22d3ee' }} aria-label="正在加载模型设置" />
        </div>}
        {isAdmin && settings && <div className="glass-card p-6 md:p-8">
          <div className="mb-7 flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl" style={{ background: 'rgba(109,40,217,0.25)', color: '#a78bfa' }}>
              <Settings size={20} />
            </div>
            <div>
              <h1 className="text-xl font-semibold" style={{ color: '#e2e8f0' }}>模型设置</h1>
              <p className="text-sm" style={{ color: '#64748b' }}>更改后续解析与角色对话使用的模型</p>
            </div>
          </div>

          <form onSubmit={submit} className="space-y-6">
            <fieldset>
              <legend className="mb-2 text-sm font-semibold" style={{ color: '#cbd5e1' }}>服务商</legend>
              <div className="grid grid-cols-2 gap-3">
                {(['deepseek', 'openai'] as const).map(provider => (
                  <button key={provider} type="button" aria-pressed={settings.provider === provider} onClick={() => selectProvider(provider)} className="rounded-lg px-4 py-3 text-left text-sm font-semibold" style={{ background: settings.provider === provider ? 'rgba(109,40,217,0.3)' : 'rgba(15,21,53,0.7)', border: `1px solid ${settings.provider === provider ? '#8b5cf6' : 'rgba(109,40,217,0.25)'}`, color: '#e2e8f0' }}>
                    {provider === 'deepseek' ? 'DeepSeek' : 'OpenAI'}
                  </button>
                ))}
              </div>
            </fieldset>

            <label className="block text-sm font-semibold" style={{ color: '#cbd5e1' }}>
              模型
              <select value={settings.model} onChange={event => setSettings({ ...settings, model: event.target.value })} className="mt-2 w-full rounded-lg px-4 py-3 outline-none" style={{ background: 'rgba(15,21,53,0.8)', border: '1px solid rgba(109,40,217,0.3)', color: '#e2e8f0' }}>
                {models.map(model => <option key={model.id} value={model.id}>{model.label} — {model.hint}</option>)}
              </select>
            </label>

            {settings.provider === 'deepseek' && (
              <label className="flex cursor-pointer items-start justify-between gap-4 rounded-xl p-4" style={{ background: 'rgba(6,182,212,0.08)', border: '1px solid rgba(6,182,212,0.2)' }}>
                <span className="flex gap-3">
                  <Brain size={20} style={{ color: '#22d3ee' }} />
                  <span>
                    <span className="block text-sm font-semibold" style={{ color: '#e2e8f0' }}>角色对话启用思考模式</span>
                    <span className="mt-1 block text-xs leading-relaxed" style={{ color: '#64748b' }}>启用时通过 DeepSeek Responses API 处理推理与输出；小说 JSON 解析始终使用非思考模式，避免推理耗尽输出预算。</span>
                  </span>
                </span>
                <input type="checkbox" checked={settings.thinking_enabled} onChange={event => setSettings({ ...settings, thinking_enabled: event.target.checked })} className="mt-1 h-4 w-4" />
              </label>
            )}

            <label className="block text-sm font-semibold" style={{ color: '#cbd5e1' }}>
              <span className="flex items-center gap-2"><Key size={15} /> API Key（留空则保持现有 Key）</span>
              <input type="password" value={apiKey} onChange={event => setApiKey(event.target.value)} autoComplete="off" placeholder={settings.api_key_configured ? '已配置' : '请输入 API Key'} className="mt-2 w-full rounded-lg px-4 py-3 outline-none" style={{ background: 'rgba(15,21,53,0.8)', border: '1px solid rgba(109,40,217,0.3)', color: '#e2e8f0' }} />
            </label>

            <button type="submit" disabled={saving} className="flex w-full items-center justify-center gap-2 rounded-lg py-3 font-semibold" style={{ background: 'linear-gradient(135deg, #0891b2, #6d28d9)', color: 'white', opacity: saving ? 0.65 : 1 }}>
              {saving ? <Loader2 size={16} className="animate-spin" /> : <Save size={16} />}
              {saving ? '正在验证并保存…' : '保存模型设置'}
            </button>
          </form>
        </div>}

        <section className="glass-card mt-6 p-6 md:p-8" aria-labelledby="account-settings-heading">
          <div className="mb-5 flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl" style={{ background: 'rgba(239,68,68,0.12)', color: '#f87171' }}>
              <Trash2 size={20} />
            </div>
            <div>
              <h1 id="account-settings-heading" className="text-xl font-semibold" style={{ color: '#e2e8f0' }}>账号设置</h1>
              <p className="text-sm" style={{ color: '#64748b' }}>{user?.email}</p>
            </div>
          </div>
          <p className="mb-5 text-sm leading-relaxed" style={{ color: '#94a3b8' }}>
            删除后，NovelWorld 保存的小说正文、对话、记忆、世界模型与个人时间线将永久移除。模型服务商可能保留的数据受其政策约束。
          </p>
          <button type="button" disabled={deleting} onClick={eraseAccount} className="flex w-full items-center justify-center gap-2 rounded-lg py-3 font-semibold" style={{ background: 'rgba(127,29,29,0.5)', border: '1px solid rgba(248,113,113,0.5)', color: '#fecaca', opacity: deleting ? 0.65 : 1 }}>
            {deleting ? <Loader2 size={16} className="animate-spin" /> : <Trash2 size={16} />}
            {deleting ? '正在删除…' : '删除账号'}
          </button>
        </section>
      </div>
    </main>
  );
}
