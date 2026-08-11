import { useState, type FormEvent } from 'react';
import { AlertCircle, BookOpen, Loader2, User } from 'lucide-react';
import { apiClient, getApiErrorMessage } from '@/shared/api/client';

export function SetupPage({ onComplete }: { onComplete: () => void }) {
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
        // ponytail: remove after the pre-H1 setup client is outside the rollout window.
        provider: 'runtime-configured',
        api_key: '',
      });
      localStorage.setItem('auth_token', response.data.access_token);
      localStorage.setItem('refresh_token', response.data.refresh_token);
      onComplete();
    } catch (requestError: unknown) {
      setError(getApiErrorMessage(requestError, 'Setup failed. Please try again.'));
      setSubmitting(false);
    }
  };

  return (
    <main
      className="min-h-screen flex items-center justify-center px-4"
      style={{ background: 'linear-gradient(135deg, var(--color-void) 0%, var(--color-cosmos) 100%)' }}
    >
      <div className="w-full max-w-md">
        <div className="text-center mb-8">
          <BookOpen size={36} className="mx-auto mb-3" style={{ color: 'var(--color-nova-glow)' }} />
          <h1 className="text-3xl" style={{ fontFamily: 'var(--font-display)', color: 'var(--color-nova-glow)' }}>
            NovelWorld
          </h1>
          <p className="mt-2" style={{ color: 'var(--color-moonbeam)' }}>
            Create the first administrator account. AI credentials are read from the server environment.
          </p>
        </div>

        <form
          onSubmit={finishSetup}
          className="rounded-xl p-8 space-y-4"
          style={{
            background: 'rgba(15, 21, 53, 0.8)',
            border: '1px solid rgba(109, 40, 217, 0.3)',
            backdropFilter: 'blur(20px)',
          }}
        >
          <div className="flex items-center gap-2 mb-2">
            <User size={20} style={{ color: 'var(--color-aurora-light)' }} />
            <h2 className="text-lg font-semibold" style={{ color: 'var(--color-starlight)' }}>
              Administrator account
            </h2>
          </div>

          <label className="block text-sm" style={{ color: 'var(--color-moonbeam)' }}>
            Display name (optional)
            <input
              value={name}
              onChange={(event) => setName(event.target.value)}
              maxLength={200}
              autoComplete="name"
              className="mt-1 w-full px-4 py-3 rounded-lg outline-none"
              style={inputStyle}
            />
          </label>

          <label className="block text-sm" style={{ color: 'var(--color-moonbeam)' }}>
            Email
            <input
              type="email"
              value={email}
              onChange={(event) => setEmail(event.target.value)}
              maxLength={320}
              autoComplete="email"
              required
              className="mt-1 w-full px-4 py-3 rounded-lg outline-none"
              style={inputStyle}
            />
          </label>

          <label className="block text-sm" style={{ color: 'var(--color-moonbeam)' }}>
            Password
            <input
              type="password"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              minLength={8}
              autoComplete="new-password"
              required
              className="mt-1 w-full px-4 py-3 rounded-lg outline-none"
              style={inputStyle}
            />
          </label>

          {error && (
            <div role="alert" className="flex gap-2 rounded-lg p-3 text-sm" style={{ background: 'rgba(239, 68, 68, 0.12)', color: '#fca5a5' }}>
              <AlertCircle size={18} aria-hidden="true" />
              <span>{error}</span>
            </div>
          )}

          <button
            type="submit"
            disabled={submitting}
            className="w-full py-3 rounded-lg font-semibold flex items-center justify-center gap-2"
            style={{
              background: 'linear-gradient(135deg, var(--color-aurora), var(--color-nova))',
              color: 'white',
              opacity: submitting ? 0.7 : 1,
            }}
          >
            {submitting && <Loader2 size={16} className="animate-spin" aria-hidden="true" />}
            {submitting ? 'Creating account…' : 'Create administrator'}
          </button>
        </form>
      </div>
    </main>
  );
}

const inputStyle = {
  background: 'rgba(3, 4, 10, 0.6)',
  border: '1px solid rgba(109, 40, 217, 0.2)',
  color: 'var(--color-starlight)',
};
