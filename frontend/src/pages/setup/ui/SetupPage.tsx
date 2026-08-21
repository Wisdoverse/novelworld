import { useState, type FormEvent, type ReactNode } from 'react';
import {
  AlertCircle,
  BookOpen,
  Check,
  ChevronLeft,
  ChevronRight,
  KeyRound,
  Loader2,
  ShieldCheck,
  UserRound,
} from 'lucide-react';
import { apiClient, getApiErrorMessage } from '@/shared/api/client';

const PROVIDERS = [
  { id: 'deepseek', name: 'DeepSeek', hint: '推荐，中文小说性价比高', keyUrl: 'https://platform.deepseek.com/api_keys' },
  { id: 'openai', name: 'OpenAI', hint: '使用 GPT-4o mini', keyUrl: 'https://platform.openai.com/api-keys' },
] as const;

const steps = [
  { number: 1, label: '连接 AI 模型' },
  { number: 2, label: '创建管理员' },
];

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
    <main className="min-h-screen bg-[#f7f8fc] px-4 py-6 text-[#1f1f1f] sm:px-6 sm:py-10 lg:flex lg:items-center">
      <div className="mx-auto w-full max-w-5xl overflow-hidden rounded-[28px] border border-[#e1e3e8] bg-white shadow-[0_12px_40px_rgba(60,64,67,0.10)]">
        <div className="grid lg:min-h-[650px] lg:grid-cols-[0.92fr_1.08fr]">
          <aside className="flex flex-col bg-[#f0f4ff] p-6 sm:p-10 lg:p-12">
            <div className="flex items-center gap-3 text-[15px] font-semibold text-[#174ea6]">
              <span className="flex h-10 w-10 items-center justify-center rounded-xl bg-[#0b57d0] text-white shadow-sm">
                <BookOpen size={21} aria-hidden="true" />
              </span>
              NovelWorld
            </div>

            <div className="mt-8 lg:mt-16">
              <p className="mb-3 text-sm font-medium text-[#0b57d0]">首次设置</p>
              <h1 className="max-w-md text-[2rem] font-medium leading-tight tracking-[-0.025em] text-[#1f1f1f] sm:text-[2.5rem]">
                欢迎使用 NovelWorld
              </h1>
              <p className="mt-3 max-w-sm text-base leading-7 text-[#5f6368] sm:mt-5">
                用两步完成基础配置，然后把小说变成一个可以亲自进入的世界。
              </p>
            </div>

            <nav className="mt-7 grid grid-cols-2 gap-2 lg:mt-auto lg:block lg:space-y-3" aria-label={`设置进度：第 ${step} 步，共 2 步`}>
              {steps.map(item => {
                const active = item.number === step;
                const complete = item.number < step || (llmConfigured && item.number === 1);
                return (
                  <div
                    key={item.number}
                    className={`flex items-center gap-2 rounded-2xl px-3 py-3 transition-colors lg:gap-3 lg:px-4 ${active ? 'bg-white shadow-sm' : ''}`}
                    aria-current={active ? 'step' : undefined}
                  >
                    <span
                      className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-sm font-semibold ${
                        active || complete ? 'bg-[#0b57d0] text-white' : 'bg-[#dfe3ea] text-[#5f6368]'
                      }`}
                    >
                      {complete ? <Check size={16} aria-hidden="true" /> : item.number}
                    </span>
                    <span className={`text-xs sm:text-sm ${active ? 'font-semibold text-[#1f1f1f]' : 'text-[#5f6368]'}`}>
                      {item.label}
                    </span>
                  </div>
                );
              })}
            </nav>
          </aside>

          <section className="flex flex-col justify-center p-7 sm:p-10 lg:p-14">
            {step === 1 ? (
              <div>
                <StepHeading
                  icon={<KeyRound size={20} aria-hidden="true" />}
                  eyebrow="第 1 步（共 2 步）"
                  title="连接你的 AI 模型"
                  description="选择提供商并填写 API Key。之后可以随时在设置中更改。"
                />

                <div className="mt-7 grid gap-3 sm:grid-cols-2" role="group" aria-label="AI 模型提供商">
                  {PROVIDERS.map(item => {
                    const selected = provider === item.id;
                    return (
                      <button
                        key={item.id}
                        type="button"
                        aria-pressed={selected}
                        onClick={() => setProvider(item.id)}
                        className={`min-h-[92px] rounded-2xl border p-4 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#0b57d0] focus-visible:ring-offset-2 ${
                          selected
                            ? 'border-[#0b57d0] bg-[#eef4ff]'
                            : 'border-[#dadce0] bg-white hover:bg-[#f8fafd]'
                        }`}
                      >
                        <span className="flex items-center justify-between gap-3">
                          <span className="text-[15px] font-semibold text-[#1f1f1f]">{item.name}</span>
                          <span
                            className={`flex h-5 w-5 items-center justify-center rounded-full border ${
                              selected ? 'border-[#0b57d0] bg-[#0b57d0] text-white' : 'border-[#9aa0a6]'
                            }`}
                          >
                            {selected ? <Check size={13} aria-hidden="true" /> : null}
                          </span>
                        </span>
                        <span className="mt-2 block text-xs leading-5 text-[#5f6368]">{item.hint}</span>
                      </button>
                    );
                  })}
                </div>

                <label className="mt-6 block text-sm font-medium text-[#3c4043]">
                  API Key
                  <input
                    type="password"
                    value={apiKey}
                    onChange={event => setApiKey(event.target.value)}
                    autoComplete="off"
                    required
                    placeholder="请输入 API Key"
                    className={inputClassName}
                  />
                </label>

                <div className="mt-3 flex items-start gap-2 text-xs leading-5 text-[#5f6368]">
                  <ShieldCheck size={16} className="mt-0.5 shrink-0 text-[#188038]" aria-hidden="true" />
                  <p>
                    Key 仅发送到你的服务器并加密保存，不会写入浏览器存储。
                    {' '}
                    <a
                      href={PROVIDERS.find(item => item.id === provider)?.keyUrl}
                      target="_blank"
                      rel="noreferrer"
                      className="font-medium text-[#0b57d0] hover:underline"
                    >
                      前往官方创建 Key
                    </a>
                  </p>
                </div>

                <div className="mt-8 flex justify-end">
                  <button
                    type="button"
                    disabled={!apiKey.trim()}
                    onClick={() => setStep(2)}
                    className={primaryButtonClassName}
                  >
                    下一步 <ChevronRight size={18} aria-hidden="true" />
                  </button>
                </div>
              </div>
            ) : (
              <form onSubmit={finishSetup}>
                <StepHeading
                  icon={<UserRound size={20} aria-hidden="true" />}
                  eyebrow="第 2 步（共 2 步）"
                  title="创建管理员账户"
                  description="这个账户用于管理书库、模型与服务设置。"
                />

                <div className="mt-7 space-y-5">
                  <label className="block text-sm font-medium text-[#3c4043]">
                    昵称（可选）
                    <input value={name} onChange={event => setName(event.target.value)} maxLength={200} autoComplete="name" placeholder="如何称呼你" className={inputClassName} />
                  </label>
                  <label className="block text-sm font-medium text-[#3c4043]">
                    邮箱
                    <input type="email" value={email} onChange={event => setEmail(event.target.value)} maxLength={320} autoComplete="email" required placeholder="name@example.com" className={inputClassName} />
                  </label>
                  <label className="block text-sm font-medium text-[#3c4043]">
                    密码（至少 8 位）
                    <input type="password" value={password} onChange={event => setPassword(event.target.value)} minLength={8} autoComplete="new-password" required placeholder="请输入密码" className={inputClassName} />
                  </label>
                </div>

                {error ? (
                  <div role="alert" className="mt-5 flex gap-2 rounded-xl bg-[#fce8e6] p-3.5 text-sm text-[#b3261e]">
                    <AlertCircle size={18} className="mt-0.5 shrink-0" aria-hidden="true" />
                    <span>{error}</span>
                  </div>
                ) : null}

                <div className="mt-8 flex items-center justify-between gap-3">
                  {!llmConfigured ? (
                    <button
                      type="button"
                      onClick={() => { setError(''); setStep(1); }}
                      className="inline-flex min-h-11 items-center gap-1 rounded-full px-4 font-semibold text-[#0b57d0] hover:bg-[#eef4ff] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#0b57d0]"
                    >
                      <ChevronLeft size={18} aria-hidden="true" /> 返回
                    </button>
                  ) : <span />}
                  <button type="submit" disabled={submitting} className={primaryButtonClassName}>
                    {submitting ? <Loader2 size={17} className="animate-spin" aria-hidden="true" /> : null}
                    {submitting ? '正在验证并保存…' : '完成设置'}
                  </button>
                </div>
              </form>
            )}
          </section>
        </div>
      </div>
    </main>
  );
}

function StepHeading({
  icon,
  eyebrow,
  title,
  description,
}: {
  icon: ReactNode;
  eyebrow: string;
  title: string;
  description: string;
}) {
  return (
    <header>
      <div className="flex items-center gap-2 text-sm font-medium text-[#0b57d0]">
        {icon}
        <span>{eyebrow}</span>
      </div>
      <h2 className="mt-4 text-2xl font-medium tracking-[-0.015em] text-[#1f1f1f] sm:text-[1.75rem]">{title}</h2>
      <p className="mt-2 max-w-lg text-sm leading-6 text-[#5f6368]">{description}</p>
    </header>
  );
}

const inputClassName = 'mt-2 w-full rounded-xl border border-[#9aa0a6] bg-white px-4 py-3 text-base text-[#1f1f1f] outline-none transition-shadow placeholder:text-[#9aa0a6] hover:border-[#5f6368] focus:border-[#0b57d0] focus:ring-1 focus:ring-[#0b57d0]';

const primaryButtonClassName = 'inline-flex min-h-11 items-center justify-center gap-1.5 rounded-full bg-[#0b57d0] px-6 font-semibold text-white shadow-sm transition-colors hover:bg-[#0842a0] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#0b57d0] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:bg-[#c4c7c5] disabled:text-[#747775] disabled:shadow-none';
