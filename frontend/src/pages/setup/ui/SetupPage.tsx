import { useState, type FormEvent } from 'react';
import { AlertCircle, BookOpen, ChevronLeft, ChevronRight, Key, Loader2, User } from 'lucide-react';
import { apiClient, getApiErrorMessage } from '@/shared/api/client';

const PROVIDERS = [
  { id: 'deepseek', name: 'DeepSeek', hint: '推荐，中文小说性价比高', keyUrl: 'https://platform.deepseek.com/api_keys' },
  { id: 'openai', name: 'OpenAI', hint: '使用 GPT-4o mini', keyUrl: 'https://platform.openai.com/api-keys' },
] as const;

export function SetupPage({
  onComplete,
  llmConfigured,
}: {
  onComplete: () => void;
  llmConfigured: boolean;
}) {
  const [step, setStep] = useState(llmConfigured ? 2 : 1);
  const [provider, setProvider] = useState('deepseek');
  const [apiKey, setApiKey] = useState('');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [name, setName] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState('');

  const finishSetup = async (event: FormEvent) => {
    event.preventDefault();
    setSubmitting(true);
    setError('');
    try {
      const response = await apiClient.post('/setup/init', {
        email,
        password,
        name: name || undefined,
        ...(llmConfigured ? {} : { provider, api_key: apiKey }),
      });
      localStorage.setItem('auth_token', response.data.access_token);
      localStorage.setItem('refresh_token', response.data.refresh_token);
      setApiKey('');
      onComplete();
    } catch (requestError: unknown) {
      setError(getApiErrorMessage(requestError, '设置失败，请检查后重试。'));
      setSubmitting(false);
    }
  };

  return (
    <main
      className="min-h-screen flex items-center justify-center px-4 py-8"
      style={{ background: 'linear-gradient(135deg, var(--color-void) 0%, var(--color-cosmos) 100%)' }}
    >
      <div className="w-full max-w-md">
        <div className="text-center mb-7">
          <BookOpen size={36} className="mx-auto mb-3" style={{ color: 'var(--color-nova-glow)' }} />
          <h1 className="text-3xl" style={{ fontFamily: 'var(--font-display)', color: 'var(--color-nova-glow)' }}>
            欢迎使用 NovelWorld
          </h1>
          <p className="mt-2" style={{ color: 'var(--color-moonbeam)' }}>
            首次使用只需完成两步设置
          </p>
        </div>

        <div className="flex items-center justify-center gap-3 mb-5" aria-label={`设置进度：第 ${step} 步，共 2 步`}>
          {[1, 2].map(number => (
            <div
              key={number}
              className="w-8 h-8 rounded-full flex items-center justify-center text-sm font-bold"
              style={{
                background: step >= number ? '#0e7490' : 'rgba(15, 21, 53, 0.8)',
                color: step >= number ? 'white' : 'var(--color-comet)',
                border: '1px solid rgba(109, 40, 217, 0.4)',
              }}
            >
              {number}
            </div>
          ))}
        </div>

        <section className="rounded-xl p-7" style={cardStyle}>
          {step === 1 ? (
            <div>
              <div className="flex items-center gap-2 mb-5">
                <Key size={20} aria-hidden="true" style={{ color: 'var(--color-aurora-light)' }} />
                <h2 className="text-lg font-semibold" style={{ color: 'var(--color-starlight)' }}>
                  第 1 步：连接 AI 模型
                </h2>
              </div>

              <div className="grid grid-cols-2 gap-3 mb-4">
                {PROVIDERS.map(item => (
                  <button
                    key={item.id}
                    type="button"
                    aria-pressed={provider === item.id}
                    onClick={() => setProvider(item.id)}
                    className="p-3 rounded-lg text-left"
                    style={{
                      background: provider === item.id ? 'rgba(109, 40, 217, 0.3)' : 'rgba(3, 4, 10, 0.4)',
                      border: `1px solid ${provider === item.id ? 'rgba(139, 92, 246, 0.8)' : 'rgba(109, 40, 217, 0.2)'}`,
                    }}
                  >
                    <span className="block text-sm font-semibold" style={{ color: 'var(--color-starlight)' }}>{item.name}</span>
                    <span className="block mt-1 text-xs" style={{ color: 'var(--color-comet)' }}>{item.hint}</span>
                  </button>
                ))}
              </div>

              <label className="block text-sm" style={{ color: 'var(--color-moonbeam)' }}>
                API Key
                <input
                  type="password"
                  value={apiKey}
                  onChange={event => setApiKey(event.target.value)}
                  autoComplete="off"
                  required
                  className="mt-1 w-full px-4 py-3 rounded-lg outline-none"
                  style={inputStyle}
                />
              </label>
              <p className="mt-2 text-xs" style={{ color: 'var(--color-comet)' }}>
                Key 仅发送到你的服务器，加密保存，不会写入浏览器存储。
                {' '}
                <a
                  href={PROVIDERS.find(item => item.id === provider)?.keyUrl}
                  target="_blank"
                  rel="noreferrer"
                  className="underline"
                  style={{ color: 'var(--color-aurora-light)' }}
                >
                  没有 Key？前往官方创建
                </a>
              </p>

              <button
                type="button"
                disabled={!apiKey.trim()}
                onClick={() => setStep(2)}
                className="mt-5 w-full py-3 rounded-lg font-semibold flex items-center justify-center gap-1"
                style={{ ...primaryButtonStyle, opacity: apiKey.trim() ? 1 : 0.5 }}
              >
                下一步 <ChevronRight size={17} aria-hidden="true" />
              </button>
            </div>
          ) : (
            <form onSubmit={finishSetup} className="space-y-4">
              <div className="flex items-center gap-2 mb-2">
                <User size={20} aria-hidden="true" style={{ color: 'var(--color-aurora-light)' }} />
                <h2 className="text-lg font-semibold" style={{ color: 'var(--color-starlight)' }}>
                  第 2 步：创建管理员
                </h2>
              </div>

              <label className="block text-sm" style={{ color: 'var(--color-moonbeam)' }}>
                昵称（可选）
                <input value={name} onChange={event => setName(event.target.value)} maxLength={200} autoComplete="name" className="mt-1 w-full px-4 py-3 rounded-lg outline-none" style={inputStyle} />
              </label>
              <label className="block text-sm" style={{ color: 'var(--color-moonbeam)' }}>
                邮箱
                <input type="email" value={email} onChange={event => setEmail(event.target.value)} maxLength={320} autoComplete="email" required className="mt-1 w-full px-4 py-3 rounded-lg outline-none" style={inputStyle} />
              </label>
              <label className="block text-sm" style={{ color: 'var(--color-moonbeam)' }}>
                密码（至少 8 位）
                <input type="password" value={password} onChange={event => setPassword(event.target.value)} minLength={8} autoComplete="new-password" required className="mt-1 w-full px-4 py-3 rounded-lg outline-none" style={inputStyle} />
              </label>

              {error ? (
                <div role="alert" className="flex gap-2 rounded-lg p-3 text-sm" style={{ background: 'rgba(239, 68, 68, 0.12)', color: '#fca5a5' }}>
                  <AlertCircle size={18} aria-hidden="true" />
                  <span>{error}</span>
                </div>
              ) : null}

              <div className="flex gap-3 pt-1">
                {!llmConfigured ? (
                  <button type="button" onClick={() => { setError(''); setStep(1); }} className="px-4 py-3 rounded-lg flex items-center gap-1" style={secondaryButtonStyle}>
                    <ChevronLeft size={17} aria-hidden="true" /> 返回
                  </button>
                ) : null}
                <button type="submit" disabled={submitting} className="flex-1 py-3 rounded-lg font-semibold flex items-center justify-center gap-2" style={{ ...primaryButtonStyle, opacity: submitting ? 0.7 : 1 }}>
                  {submitting ? <Loader2 size={16} className="animate-spin" aria-hidden="true" /> : null}
                  {submitting ? '正在验证并保存…' : '完成设置'}
                </button>
              </div>
            </form>
          )}
        </section>
      </div>
    </main>
  );
}

const cardStyle = {
  background: 'rgba(15, 21, 53, 0.8)',
  border: '1px solid rgba(109, 40, 217, 0.3)',
  backdropFilter: 'blur(20px)',
};

const inputStyle = {
  background: 'rgba(3, 4, 10, 0.6)',
  border: '1px solid rgba(109, 40, 217, 0.2)',
  color: 'var(--color-starlight)',
};

const primaryButtonStyle = {
  background: 'linear-gradient(135deg, var(--color-aurora), var(--color-nova))',
  color: 'white',
};

const secondaryButtonStyle = {
  background: 'rgba(3, 4, 10, 0.4)',
  border: '1px solid rgba(109, 40, 217, 0.2)',
  color: 'var(--color-moonbeam)',
};
