import {
  ArrowRight,
  BookOpen,
  CheckCircle2,
  GitBranch,
  MessageCircle,
  Sparkles,
  Upload,
  UserRound,
} from 'lucide-react';
import { useNavigate } from 'react-router-dom';
import { useAuthStore } from '@/features/auth/model/useAuthStore';

const capabilities = [
  {
    icon: Upload,
    title: '导入你的小说',
    description: '支持 TXT、EPUB 和 PDF，自动整理章节、角色与世界设定。',
  },
  {
    icon: UserRound,
    title: '以玩家身份进入',
    description: '你不是旁观者，而是带着自己的身份加入小说世界。',
  },
  {
    icon: GitBranch,
    title: '让故事回应选择',
    description: '每次决定都会改变世界状态，并重新生成之后的章节。',
  },
];

export function HomePage() {
  const navigate = useNavigate();
  const user = useAuthStore(state => state.user);
  const startDestination = user ? '/shelf' : '/register';

  return (
    <main className="app-surface min-h-screen">
      <header className="border-b border-[#e8eaed] bg-white/90 backdrop-blur-xl">
        <div className="mx-auto flex min-h-16 max-w-7xl items-center justify-between px-4 sm:px-6 lg:px-8">
          <button
            type="button"
            onClick={() => navigate('/')}
            className="flex items-center gap-3 rounded-full pr-3 text-left"
            aria-label="NovelWorld 首页"
          >
            <span className="flex h-10 w-10 items-center justify-center rounded-xl bg-[#0b57d0] text-white">
              <BookOpen size={18} aria-hidden="true" />
            </span>
            <span className="font-semibold tracking-[-0.01em] text-[#174ea6]">NovelWorld</span>
          </button>

          <div className="flex items-center gap-2">
            {!user && (
              <button
                type="button"
                onClick={() => navigate('/login')}
                className="flex min-h-11 items-center rounded-full px-4 text-sm font-semibold text-[#0b57d0] transition-colors hover:bg-[#f0f4ff]"
              >
                登录
              </button>
            )}
            <button
              type="button"
              onClick={() => navigate(startDestination)}
              className="primary-action px-5 text-sm"
            >
              {user ? '进入书架' : '免费开始'}
            </button>
          </div>
        </div>
      </header>

      <section className="mx-auto grid max-w-7xl items-center gap-12 px-4 py-14 sm:px-6 sm:py-20 lg:grid-cols-[1.04fr_0.96fr] lg:px-8 lg:py-24">
        <div className="max-w-2xl">
          <div className="mb-6 inline-flex items-center gap-2 rounded-full bg-[#e8f0fe] px-4 py-2 text-sm font-semibold text-[#174ea6]">
            <Sparkles size={15} aria-hidden="true" />
            小说，从阅读变成亲历
          </div>

          <h1 className="text-balance text-5xl font-medium leading-[1.08] tracking-[-0.045em] text-[#1f1f1f] sm:text-6xl lg:text-7xl">
            进入故事，
            <span className="text-[#0b57d0]">成为其中的玩家。</span>
          </h1>
          <p className="mt-7 max-w-xl text-lg leading-8 text-[#5f6368] sm:text-xl">
            导入一本小说，NovelWorld 会理解角色、关系和世界规则。你做出选择，故事从这一刻开始为你继续书写。
          </p>

          <div className="mt-9 flex flex-col gap-3 sm:flex-row">
            <button
              type="button"
              onClick={() => navigate(startDestination)}
              className="primary-action min-h-12 px-7 text-base"
            >
              开始你的旅程
              <ArrowRight size={18} aria-hidden="true" />
            </button>
            <button
              type="button"
              onClick={() => navigate(user ? '/shelf' : '/login')}
              className="tonal-action min-h-12 px-7 text-base"
            >
              {user ? '打开我的书架' : '我已有账号'}
            </button>
          </div>

          <p className="mt-5 text-sm text-[#5f6368]">支持中文与英文小说 · 保留原著设定 · 每位玩家拥有独立世界线</p>
        </div>

        <div className="rounded-[32px] bg-[#e8f0fe] p-3 sm:p-6" aria-label="交互式章节示例">
          <div className="overflow-hidden rounded-[24px] border border-[#d2e3fc] bg-white shadow-[0_18px_50px_rgba(60,64,67,0.14)]">
            <div className="flex items-center justify-between border-b border-[#e8eaed] px-5 py-4 sm:px-6">
              <div>
                <p className="text-xs font-semibold uppercase tracking-[0.12em] text-[#5f6368]">第六章</p>
                <h2 className="mt-1 font-semibold text-[#1f1f1f]">雨夜来客</h2>
              </div>
              <span className="rounded-full bg-[#e6f4ea] px-3 py-1 text-xs font-semibold text-[#137333]">你的世界线</span>
            </div>

            <div className="px-5 py-6 sm:px-7 sm:py-8">
              <p className="font-[var(--font-reading)] text-base leading-8 text-[#3c4043]">
                门外传来第三次敲门声。原著中的主角此刻应该离开，但你知道，门后的人握着改变整座城命运的线索。
              </p>

              <div className="mt-7 rounded-2xl bg-[#f8fafd] p-4 sm:p-5">
                <div className="mb-4 flex items-center gap-2 text-sm font-semibold text-[#1f1f1f]">
                  <MessageCircle size={17} className="text-[#0b57d0]" aria-hidden="true" />
                  你准备怎么做？
                </div>
                <div className="space-y-2.5">
                  <div className="flex items-center gap-3 rounded-xl border border-[#0b57d0] bg-[#e8f0fe] px-4 py-3 text-sm font-medium text-[#174ea6]">
                    <CheckCircle2 size={17} className="shrink-0" aria-hidden="true" />
                    打开门，先听听来客想说什么
                  </div>
                  <div className="rounded-xl border border-[#dadce0] bg-white px-4 py-3 text-sm text-[#5f6368]">
                    按照原著路线，从后门离开
                  </div>
                </div>
              </div>
            </div>

            <div className="flex items-center gap-2 border-t border-[#e8eaed] bg-[#f8fafd] px-5 py-4 text-sm text-[#5f6368] sm:px-7">
              <Sparkles size={15} className="shrink-0 text-[#0b57d0]" aria-hidden="true" />
              选择后，后续章节会基于你的世界线重新生成
            </div>
          </div>
        </div>
      </section>

      <section className="border-y border-[#e8eaed] bg-white" aria-labelledby="capabilities-heading">
        <div className="mx-auto max-w-7xl px-4 py-14 sm:px-6 lg:px-8 lg:py-18">
          <div className="max-w-2xl">
            <p className="text-sm font-semibold text-[#0b57d0]">从一本书开始</p>
            <h2 id="capabilities-heading" className="mt-2 text-3xl font-medium tracking-[-0.03em] text-[#1f1f1f] sm:text-4xl">
              阅读、选择与生成，发生在同一个世界里。
            </h2>
          </div>

          <div className="mt-10 grid gap-4 md:grid-cols-3">
            {capabilities.map(({ icon: Icon, title, description }) => (
              <article key={title} className="rounded-3xl border border-[#e1e3e8] bg-[#f8fafd] p-6 sm:p-7">
                <span className="flex h-11 w-11 items-center justify-center rounded-2xl bg-[#e8f0fe] text-[#0b57d0]">
                  <Icon size={20} aria-hidden="true" />
                </span>
                <h3 className="mt-6 text-lg font-semibold text-[#1f1f1f]">{title}</h3>
                <p className="mt-2 text-sm leading-6 text-[#5f6368]">{description}</p>
              </article>
            ))}
          </div>
        </div>
      </section>

      <footer className="bg-white">
        <div className="mx-auto flex max-w-7xl flex-col gap-2 px-4 py-7 text-sm text-[#747775] sm:flex-row sm:items-center sm:justify-between sm:px-6 lg:px-8">
          <span className="font-semibold text-[#3c4043]">NovelWorld</span>
          <span>让每一本读完的书，都拥有新的开始。</span>
        </div>
      </footer>
    </main>
  );
}
