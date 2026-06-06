import React, { useEffect, useRef } from 'react';
import { useNavigate } from 'react-router-dom';
import { motion } from 'framer-motion';
import { BookOpen, Users, GitBranch, Brain, Sparkles, ArrowRight, Star } from 'lucide-react';

// 星点组件
function StarField() {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    canvas.width = window.innerWidth;
    canvas.height = window.innerHeight;

    const stars = Array.from({ length: 200 }, () => ({
      x: Math.random() * canvas.width,
      y: Math.random() * canvas.height,
      r: Math.random() * 1.5 + 0.3,
      opacity: Math.random() * 0.8 + 0.2,
      speed: Math.random() * 0.5 + 0.1,
    }));

    let animId: number;
    const animate = () => {
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      stars.forEach((star) => {
        ctx.beginPath();
        ctx.arc(star.x, star.y, star.r, 0, Math.PI * 2);
        ctx.fillStyle = `rgba(255, 255, 255, ${star.opacity})`;
        ctx.fill();
        star.opacity += (Math.random() - 0.5) * 0.02;
        star.opacity = Math.max(0.1, Math.min(1, star.opacity));
      });
      animId = requestAnimationFrame(animate);
    };
    animate();

    const handleResize = () => {
      canvas.width = window.innerWidth;
      canvas.height = window.innerHeight;
    };
    window.addEventListener('resize', handleResize);

    return () => {
      cancelAnimationFrame(animId);
      window.removeEventListener('resize', handleResize);
    };
  }, []);

  return (
    <canvas
      ref={canvasRef}
      className="fixed inset-0 z-0 pointer-events-none"
      style={{ opacity: 0.6 }}
    />
  );
}

const features = [
  {
    icon: BookOpen,
    title: '任意小说导入',
    desc: '上传 TXT/PDF 或粘贴文本，AI 自动解析章节、提取角色、构建世界观',
    color: '#22d3ee',
  },
  {
    icon: Users,
    title: '角色 Agent 系统',
    desc: '每个角色拥有独立 AI 人格，性格、背景、说话风格完全还原原著',
    color: '#8b5cf6',
  },
  {
    icon: Brain,
    title: '4层记忆金字塔',
    desc: '角色记住你们的每次对话，关系随时间真实演进，永不遗忘',
    color: '#22d3ee',
  },
  {
    icon: GitBranch,
    title: '分支叙事引擎',
    desc: '在关键节点做出选择，AI 动态生成专属于你的故事走向',
    color: '#8b5cf6',
  },
  {
    icon: Sparkles,
    title: '身份代入系统',
    desc: '以自己身份进入，或扮演书中角色，深度参与故事世界',
    color: '#22d3ee',
  },
  {
    icon: Star,
    title: '角色头像生成',
    desc: '根据原著外貌描述，AI 自动生成每个角色的专属插图头像',
    color: '#8b5cf6',
  },
];

export function HomePage() {
  const navigate = useNavigate();

  return (
    <div className="relative min-h-screen overflow-hidden" style={{ background: 'var(--color-void)' }}>
      <StarField />

      {/* 星云背景 */}
      <div className="fixed inset-0 z-0 pointer-events-none">
        <div
          className="absolute animate-nebula"
          style={{
            top: '-20%', left: '-10%', width: '60%', height: '60%',
            background: 'radial-gradient(ellipse, rgba(109,40,217,0.12) 0%, transparent 70%)',
          }}
        />
        <div
          className="absolute animate-nebula"
          style={{
            bottom: '-20%', right: '-10%', width: '50%', height: '50%',
            background: 'radial-gradient(ellipse, rgba(6,182,212,0.08) 0%, transparent 70%)',
            animationDelay: '-10s',
          }}
        />
      </div>

      {/* 导航栏 */}
      <nav className="relative z-10 flex items-center justify-between px-8 py-6">
        <motion.div
          initial={{ opacity: 0, x: -20 }}
          animate={{ opacity: 1, x: 0 }}
          className="flex items-center gap-3"
        >
          <div
            className="w-9 h-9 rounded-xl flex items-center justify-center"
            style={{ background: 'linear-gradient(135deg, #6d28d9, #06b6d4)' }}
          >
            <BookOpen size={18} color="white" />
          </div>
          <span
            className="text-xl font-bold"
            style={{ fontFamily: 'var(--font-display)', color: '#e2e8f0' }}
          >
            NovelWorld
          </span>
        </motion.div>

        <motion.div
          initial={{ opacity: 0, x: 20 }}
          animate={{ opacity: 1, x: 0 }}
          className="flex items-center gap-3"
        >
          <button
            onClick={() => navigate('/login')}
            className="px-4 py-2 text-sm rounded-lg transition-colors"
            style={{ color: '#94a3b8' }}
          >
            登录
          </button>
          <button
            onClick={() => navigate('/register')}
            className="btn-cosmic-filled px-5 py-2 rounded-lg text-sm font-semibold"
            style={{ background: 'linear-gradient(135deg, #0891b2, #6d28d9)', color: 'white', border: 'none', cursor: 'pointer' }}
          >
            免费开始
          </button>
        </motion.div>
      </nav>

      {/* Hero 区域 */}
      <div className="relative z-10 text-center px-6 pt-16 pb-24">
        <motion.div
          initial={{ opacity: 0, y: 30 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ delay: 0.1, duration: 0.6, ease: [0.23, 1, 0.32, 1] }}
        >
          <div
            className="inline-flex items-center gap-2 px-4 py-1.5 rounded-full text-xs font-semibold mb-8"
            style={{
              background: 'rgba(109, 40, 217, 0.15)',
              border: '1px solid rgba(109, 40, 217, 0.3)',
              color: '#8b5cf6',
            }}
          >
            <Sparkles size={12} />
            AI 驱动的沉浸式小说体验
          </div>

          <h1
            className="text-5xl md:text-7xl font-black mb-6 leading-tight"
            style={{ fontFamily: 'var(--font-display)' }}
          >
            <span style={{ color: '#e2e8f0' }}>进入</span>
            <br />
            <span className="text-glow">小说的世界</span>
          </h1>

          <p
            className="text-lg md:text-xl max-w-2xl mx-auto mb-10 leading-relaxed"
            style={{ color: '#94a3b8' }}
          >
            导入任意小说，与书中角色实时对话，在关键时刻做出影响故事走向的选择。
            <br />
            你不再是旁观者——你是故事的参与者。
          </p>

          <div className="flex flex-col sm:flex-row items-center justify-center gap-4">
            <button
              onClick={() => navigate('/register')}
              className="flex items-center gap-2 px-8 py-4 rounded-xl text-base font-semibold transition-all"
              style={{
                background: 'linear-gradient(135deg, #0891b2, #6d28d9)',
                color: 'white',
                boxShadow: '0 4px 30px rgba(6, 182, 212, 0.3)',
                border: 'none',
                cursor: 'pointer',
              }}
              onMouseEnter={(e) => {
                (e.currentTarget as HTMLElement).style.transform = 'translateY(-2px)';
                (e.currentTarget as HTMLElement).style.boxShadow = '0 8px 40px rgba(6, 182, 212, 0.5)';
              }}
              onMouseLeave={(e) => {
                (e.currentTarget as HTMLElement).style.transform = '';
                (e.currentTarget as HTMLElement).style.boxShadow = '0 4px 30px rgba(6, 182, 212, 0.3)';
              }}
            >
              开始你的旅程
              <ArrowRight size={18} />
            </button>
            <button
              onClick={() => navigate('/shelf')}
              className="btn-cosmic px-8 py-4 rounded-xl text-base"
              style={{ cursor: 'pointer' }}
            >
              浏览书架
            </button>
          </div>
        </motion.div>

        {/* 功能特性网格 */}
        <motion.div
          initial={{ opacity: 0, y: 40 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ delay: 0.4, duration: 0.6 }}
          className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4 max-w-5xl mx-auto mt-24"
        >
          {features.map((feature, i) => (
            <motion.div
              key={feature.title}
              initial={{ opacity: 0, y: 20 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: 0.5 + i * 0.08, duration: 0.4 }}
              className="glass-card p-6 text-left"
            >
              <div
                className="w-10 h-10 rounded-xl flex items-center justify-center mb-4"
                style={{ background: `${feature.color}20`, border: `1px solid ${feature.color}40` }}
              >
                <feature.icon size={20} style={{ color: feature.color }} />
              </div>
              <h3 className="font-semibold mb-2" style={{ color: '#e2e8f0' }}>
                {feature.title}
              </h3>
              <p className="text-sm leading-relaxed" style={{ color: '#64748b' }}>
                {feature.desc}
              </p>
            </motion.div>
          ))}
        </motion.div>
      </div>

      {/* 底部 */}
      <footer className="relative z-10 text-center py-8 border-t" style={{ borderColor: 'rgba(109,40,217,0.1)', color: '#334155' }}>
        <p className="text-xs">© 2025 NovelWorld · 让每本书都成为你的世界</p>
      </footer>
    </div>
  );
}
