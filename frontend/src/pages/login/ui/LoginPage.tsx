import React, { useState } from 'react';
import { useNavigate, Link } from 'react-router-dom';
import { useAuthStore } from '@/features/auth';
import { getApiErrorMessage } from '@/shared/api/client';
import { toast } from 'sonner';
import { ArrowRight, BookOpen } from 'lucide-react';

export function LoginPage({ initialRegister = false }: { initialRegister?: boolean }) {
  const navigate = useNavigate();
  const { login, register, loading } = useAuthStore();
  const [isRegister, setIsRegister] = useState(initialRegister);
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [name, setName] = useState('');

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    try {
      if (isRegister) {
        await register(email, password, name || undefined);
        toast.success('注册成功');
      } else {
        await login(email, password);
        toast.success('登录成功');
      }
      navigate('/shelf');
    } catch (error: unknown) {
      toast.error(getApiErrorMessage(error, '操作失败'));
    }
  };

  return (
    <main className="app-surface min-h-screen px-4 py-6 sm:px-6 sm:py-10 lg:flex lg:items-center">
      <div className="surface-card mx-auto grid w-full max-w-5xl overflow-hidden lg:min-h-[620px] lg:grid-cols-[0.9fr_1.1fr]">
        <aside className="flex flex-col bg-[#f0f4ff] p-8 sm:p-10 lg:p-12">
          <Link to="/" className="flex items-center gap-3 text-[15px] font-semibold text-[#174ea6]">
            <span className="flex h-10 w-10 items-center justify-center rounded-xl bg-[#0b57d0] text-white shadow-sm">
              <BookOpen size={21} aria-hidden="true" />
            </span>
            NovelWorld
          </Link>
          <div className="mt-12 lg:mt-auto lg:mb-auto">
            <p className="text-sm font-medium text-[#0b57d0]">你的小说世界</p>
            <h1 className="mt-4 text-3xl font-medium leading-tight tracking-[-0.025em] text-[#1f1f1f] sm:text-4xl">
              回到故事继续发生的地方
            </h1>
            <p className="mt-5 max-w-sm text-base leading-7 text-[#5f6368]">
              阅读、探索、与角色相遇。你的每一次选择都会成为新的时间线。
            </p>
          </div>
        </aside>

        <section className="flex flex-col justify-center p-8 sm:p-12 lg:p-16">
          <div className="mx-auto w-full max-w-md">
            <p className="text-sm font-medium text-[#0b57d0]">{isRegister ? '开始使用' : '欢迎回来'}</p>
            <h2 className="mt-3 text-3xl font-medium tracking-[-0.02em] text-[#1f1f1f]">
              {isRegister ? '创建账号' : '登录 NovelWorld'}
            </h2>
            <p className="mt-2 text-sm leading-6 text-[#5f6368]">
              {isRegister ? '创建账号后即可导入小说并开始探索。' : '使用你的账号继续阅读。'}
            </p>

            <form onSubmit={handleSubmit} className="mt-8 space-y-5">
              {isRegister && (
                <label className="block text-sm font-medium text-[#3c4043]">
                  昵称（可选）
                  <input type="text" value={name} onChange={(e) => setName(e.target.value)} className="field-control mt-2" placeholder="如何称呼你" autoComplete="name" />
                </label>
              )}

              <label className="block text-sm font-medium text-[#3c4043]">
                邮箱
                <input type="email" value={email} onChange={(e) => setEmail(e.target.value)} required className="field-control mt-2" placeholder="name@example.com" autoComplete="email" />
              </label>

              <label className="block text-sm font-medium text-[#3c4043]">
                密码
                <input type="password" value={password} onChange={(e) => setPassword(e.target.value)} required minLength={8} className="field-control mt-2" placeholder="至少 8 位" autoComplete={isRegister ? 'new-password' : 'current-password'} />
              </label>

              <button type="submit" disabled={loading} className="primary-action mt-2 w-full">
                {loading ? '处理中…' : isRegister ? '创建账号' : '登录'}
                {!loading ? <ArrowRight size={18} aria-hidden="true" /> : null}
              </button>
            </form>

            <p className="mt-7 text-center text-sm text-[#5f6368]">
              {isRegister ? '已有账号？' : '还没有账号？'}
              <button type="button" onClick={() => setIsRegister(!isRegister)} className="ml-1 font-semibold text-[#0b57d0] hover:underline">
                {isRegister ? '直接登录' : '创建账号'}
              </button>
            </p>
          </div>
        </section>
      </div>
    </main>
  );
}
