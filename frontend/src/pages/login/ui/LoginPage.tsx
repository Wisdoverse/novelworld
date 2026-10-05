import { translate as t, useLocale } from '@/shared/lib/i18n';
import React, { useState } from 'react';
import { useNavigate, Link } from 'react-router-dom';
import { useAuthStore } from '@/features/auth';
import { getApiErrorMessage } from '@/shared/api/client';
import { toast } from 'sonner';
import { ArrowRight, BookOpen } from 'lucide-react';
import { LanguageSwitcher } from '@/shared/ui/LanguageSwitcher';

export function LoginPage({ initialRegister = false }: { initialRegister?: boolean }) {
  useLocale();
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
        toast.success(t("Account created"));
      } else {
        await login(email, password);
        toast.success(t("Signed in"));
      }
      navigate('/shelf');
    } catch (error: unknown) {
      toast.error(getApiErrorMessage(error, t("Operation failed")));
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
            <p className="text-sm font-medium text-[#0b57d0]">{t("Your novel world")}</p>
            <h1 className="mt-4 text-3xl font-medium leading-tight tracking-[-0.025em] text-[#1f1f1f] sm:text-4xl">
              {t("Return to a story that keeps unfolding")}
            </h1>
            <p className="mt-5 max-w-sm text-base leading-7 text-[#5f6368]">
              {t("Read, explore and meet characters. Every choice becomes part of a new timeline.")}
            </p>
          </div>
        </aside>

        <section className="flex flex-col justify-center p-8 sm:p-12 lg:p-16">
          <div className="mx-auto w-full max-w-md">
            <div className="flex items-center justify-between gap-3">
              <p className="text-sm font-medium text-[#0b57d0]">{isRegister ? t("Get started") : t("Welcome back")}</p>
              <LanguageSwitcher />
            </div>
            <h2 className="mt-3 text-3xl font-medium tracking-[-0.02em] text-[#1f1f1f]">
              {isRegister ? t("Create account") : t("Sign in to NovelWorld")}
            </h2>
            <p className="mt-2 text-sm leading-6 text-[#5f6368]">
              {isRegister ? t("Create an account to import novels and start exploring.") : t("Continue reading with your account.")}
            </p>

            <form onSubmit={handleSubmit} className="mt-8 space-y-5">
              {isRegister && (
                <label className="block text-sm font-medium text-[#3c4043]">
                  {t("Display name (optional)")}
                  <input type="text" value={name} onChange={(e) => setName(e.target.value)} className="field-control mt-2" placeholder={t("What should we call you?")} autoComplete="name" />
                </label>
              )}

              <label className="block text-sm font-medium text-[#3c4043]">
                {t("Email")}
                <input type="email" value={email} onChange={(e) => setEmail(e.target.value)} required className="field-control mt-2" placeholder="name@example.com" autoComplete="email" />
              </label>

              <label className="block text-sm font-medium text-[#3c4043]">
                {t("Password")}
                <input type="password" value={password} onChange={(e) => setPassword(e.target.value)} required minLength={8} className="field-control mt-2" placeholder={t("At least 8 characters")} autoComplete={isRegister ? 'new-password' : 'current-password'} />
              </label>

              <button type="submit" disabled={loading} className="primary-action mt-2 w-full">
                {loading ? t("Working…") : isRegister ? t("Create account") : t("Sign in")}
                {!loading ? <ArrowRight size={18} aria-hidden="true" /> : null}
              </button>
            </form>

            <p className="mt-7 text-center text-sm text-[#5f6368]">
              {isRegister ? t("Already have an account?") : t("Need an account?")}
              <button type="button" onClick={() => setIsRegister(!isRegister)} className="ml-1 font-semibold text-[#0b57d0] hover:underline">
                {isRegister ? t("Sign in instead") : t("Create account")}
              </button>
            </p>
          </div>
        </section>
      </div>
    </main>
  );
}
