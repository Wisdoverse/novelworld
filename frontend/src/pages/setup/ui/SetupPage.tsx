import { useMutation } from '@tanstack/react-query';
import { type FormEvent, useState } from 'react';
import { AlertCircle, BookOpen, Loader2, UserRound } from 'lucide-react';
import { apiClient, getApiErrorMessage } from '@/shared/api/client';
import { clearPrivateQueryCache } from '@/shared/api/queryClient';
import { clearWorldTurnPendingRequests } from '@/shared/lib/worldTurnStorage';

type SetupResponse = {
  user: { id: string };
  access_token: string;
  refresh_token: string;
};

export function SetupPage({ onComplete }: { onComplete: () => void }) {
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [name, setName] = useState('');
  const setup = useMutation({
    mutationFn: () => apiClient.post<SetupResponse>('/setup/init', {
      email,
      password,
      name: name || undefined,
    }),
    onSuccess: response => {
      clearPrivateQueryCache();
      clearWorldTurnPendingRequests(response.data.user.id);
      localStorage.setItem('auth_token', response.data.access_token);
      localStorage.setItem('refresh_token', response.data.refresh_token);
      onComplete();
    },
  });

  const finishSetup = (event: FormEvent) => {
    event.preventDefault();
    setup.mutate();
  };

  return (
    <main className="min-h-screen bg-[#f7f8fc] px-4 py-6 text-[#1f1f1f] sm:px-6 sm:py-10 lg:flex lg:items-center">
      <div className="mx-auto w-full max-w-5xl overflow-hidden rounded-[28px] border border-[#e1e3e8] bg-white shadow-[0_12px_40px_rgba(60,64,67,0.10)]">
        <div className="grid lg:min-h-[620px] lg:grid-cols-[0.92fr_1.08fr]">
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
                先创建唯一的管理员账户。AI 模型可以登录后在设置中安全配置。
              </p>
            </div>
          </aside>

          <section className="flex flex-col justify-center p-7 sm:p-10 lg:p-14">
            <form onSubmit={finishSetup}>
              <header>
                <div className="flex items-center gap-2 text-sm font-medium text-[#0b57d0]">
                  <UserRound size={20} aria-hidden="true" />
                  <span>管理员账户</span>
                </div>
                <h2 className="mt-4 text-2xl font-medium tracking-[-0.015em] text-[#1f1f1f] sm:text-[1.75rem]">
                  创建管理员账户
                </h2>
                <p className="mt-2 max-w-lg text-sm leading-6 text-[#5f6368]">
                  这个账户用于管理书库、模型与服务设置。
                </p>
              </header>

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

              {setup.isError ? (
                <div role="alert" className="mt-5 flex gap-2 rounded-xl bg-[#fce8e6] p-3.5 text-sm text-[#b3261e]">
                  <AlertCircle size={18} className="mt-0.5 shrink-0" aria-hidden="true" />
                  <span>{getApiErrorMessage(setup.error, '设置失败，请检查后重试。')}</span>
                </div>
              ) : null}

              <div className="mt-8 flex justify-end">
                <button type="submit" disabled={setup.isPending} className={primaryButtonClassName}>
                  {setup.isPending ? <Loader2 size={17} className="animate-spin" aria-hidden="true" /> : null}
                  {setup.isPending ? '正在创建…' : '创建管理员并继续'}
                </button>
              </div>
            </form>
          </section>
        </div>
      </div>
    </main>
  );
}

const inputClassName = 'mt-2 w-full rounded-xl border border-[#9aa0a6] bg-white px-4 py-3 text-base text-[#1f1f1f] outline-none transition-shadow placeholder:text-[#9aa0a6] hover:border-[#5f6368] focus:border-[#0b57d0] focus:ring-1 focus:ring-[#0b57d0]';

const primaryButtonClassName = 'inline-flex min-h-11 items-center justify-center gap-1.5 rounded-full bg-[#0b57d0] px-6 font-semibold text-white shadow-sm hover:bg-[#0842a0] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#0b57d0] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:bg-[#c4c7c5] disabled:text-[#747775] disabled:shadow-none';
