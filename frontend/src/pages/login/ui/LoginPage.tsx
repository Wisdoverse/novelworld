import React, { useState } from 'react';
import { useNavigate, Link } from 'react-router-dom';
import { useAuthStore } from '@/features/auth/model/useAuthStore';
import { toast } from 'sonner';

export function LoginPage() {
  const navigate = useNavigate();
  const { login, register, loading } = useAuthStore();
  const [isRegister, setIsRegister] = useState(false);
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
    } catch (err: any) {
      toast.error(err.response?.data?.error?.message || '操作失败');
    }
  };

  return (
    <div className="min-h-screen flex items-center justify-center px-4"
         style={{ background: 'linear-gradient(135deg, var(--color-void) 0%, var(--color-cosmos) 100%)' }}>
      <div className="w-full max-w-md">
        <Link to="/" className="block text-center mb-8">
          <h1 style={{ fontFamily: 'var(--font-display)', fontSize: '2rem', color: 'var(--color-nova-glow)' }}>
            NovelWorld
          </h1>
        </Link>

        <div className="rounded-xl p-8"
             style={{
               background: 'rgba(15, 21, 53, 0.8)',
               border: '1px solid rgba(109, 40, 217, 0.3)',
               backdropFilter: 'blur(20px)',
             }}>
          <h2 className="text-xl font-semibold mb-6 text-center"
              style={{ color: 'var(--color-starlight)' }}>
            {isRegister ? '创建账号' : '登录'}
          </h2>

          <form onSubmit={handleSubmit} className="space-y-4">
            {isRegister && (
              <div>
                <label className="block text-sm mb-1" style={{ color: 'var(--color-moonbeam)' }}>昵称</label>
                <input
                  type="text"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  className="w-full px-4 py-3 rounded-lg outline-none transition-all"
                  style={{
                    background: 'rgba(3, 4, 10, 0.6)',
                    border: '1px solid rgba(109, 40, 217, 0.2)',
                    color: 'var(--color-starlight)',
                  }}
                  placeholder="你的昵称（可选）"
                />
              </div>
            )}

            <div>
              <label className="block text-sm mb-1" style={{ color: 'var(--color-moonbeam)' }}>邮箱</label>
              <input
                type="email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                required
                className="w-full px-4 py-3 rounded-lg outline-none transition-all"
                style={{
                  background: 'rgba(3, 4, 10, 0.6)',
                  border: '1px solid rgba(109, 40, 217, 0.2)',
                  color: 'var(--color-starlight)',
                }}
                placeholder="your@email.com"
              />
            </div>

            <div>
              <label className="block text-sm mb-1" style={{ color: 'var(--color-moonbeam)' }}>密码</label>
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                required
                minLength={8}
                className="w-full px-4 py-3 rounded-lg outline-none transition-all"
                style={{
                  background: 'rgba(3, 4, 10, 0.6)',
                  border: '1px solid rgba(109, 40, 217, 0.2)',
                  color: 'var(--color-starlight)',
                }}
                placeholder="至少8位"
              />
            </div>

            <button
              type="submit"
              disabled={loading}
              className="w-full py-3 rounded-lg font-semibold transition-all"
              style={{
                background: 'linear-gradient(135deg, var(--color-aurora), var(--color-nova))',
                color: 'white',
                opacity: loading ? 0.7 : 1,
              }}
            >
              {loading ? '处理中...' : isRegister ? '注册' : '登录'}
            </button>
          </form>

          <p className="mt-6 text-center text-sm" style={{ color: 'var(--color-moonbeam)' }}>
            {isRegister ? '已有账号？' : '还没有账号？'}
            <button
              onClick={() => setIsRegister(!isRegister)}
              className="ml-1 underline"
              style={{ color: 'var(--color-nova-glow)' }}
            >
              {isRegister ? '去登录' : '去注册'}
            </button>
          </p>
        </div>
      </div>
    </div>
  );
}
