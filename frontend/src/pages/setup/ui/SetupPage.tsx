import React, { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { BookOpen, Key, User, Check, Loader2, AlertCircle, ChevronRight } from 'lucide-react';
import { apiClient } from '@/shared/api/client';

const PROVIDERS = [
  { id: 'deepseek', name: 'DeepSeek', hint: 'DeepSeek-V3, R1 — 性价比最高', placeholder: 'sk-...', free: false },
  { id: 'openai', name: 'OpenAI', hint: 'GPT-4o, GPT-4', placeholder: 'sk-...', free: false },
  { id: 'qwen', name: '通义千问 Qwen', hint: 'Qwen3 — 有免费额度', placeholder: 'sk-...', free: false },
  { id: 'glm', name: 'GLM 智谱AI', hint: 'GLM-4 — 有免费额度', placeholder: '...', free: false },
  { id: 'moonshot', name: 'Moonshot / Kimi', hint: 'Moonshot-v1', placeholder: 'sk-...', free: false },
  { id: 'doubao', name: '豆包 Doubao', hint: 'Doubao-1.5', placeholder: '...', free: false },
  { id: 'anthropic', name: 'Anthropic', hint: 'Claude Opus/Sonnet', placeholder: 'sk-ant-...', free: false },
  { id: 'ollama', name: 'Ollama (Local)', hint: 'Free — runs on your machine', placeholder: '', free: true },
];

interface SetupState {
  step: number;
  provider: string;
  apiKey: string;
  email: string;
  password: string;
  name: string;
  testing: boolean;
  testResult: 'idle' | 'success' | 'error';
  testError: string;
  submitting: boolean;
}

export function SetupPage({ onComplete }: { onComplete: () => void }) {
  const [state, setState] = useState<SetupState>({
    step: 1,
    provider: '',
    apiKey: '',
    email: '',
    password: '',
    name: '',
    testing: false,
    testResult: 'idle',
    testError: '',
    submitting: false,
  });

  const set = (partial: Partial<SetupState>) => setState(s => ({ ...s, ...partial }));

  const testConnection = async () => {
    set({ testing: true, testResult: 'idle', testError: '' });
    try {
      await apiClient.post('/setup/test-llm', {
        provider: state.provider,
        api_key: state.apiKey,
      });
      set({ testing: false, testResult: 'success' });
    } catch (e: any) {
      set({
        testing: false,
        testResult: 'error',
        testError: e.response?.data?.error || 'Connection failed. Check your API key.',
      });
    }
  };

  const finishSetup = async () => {
    set({ submitting: true });
    try {
      const res = await apiClient.post('/setup/init', {
        provider: state.provider,
        api_key: state.apiKey,
        email: state.email,
        password: state.password,
        name: state.name,
      });
      if (res.data.access_token) {
        localStorage.setItem('auth_token', res.data.access_token);
        if (res.data.refresh_token) {
          localStorage.setItem('refresh_token', res.data.refresh_token);
        }
      }
      set({ step: 4 });
      setTimeout(() => onComplete(), 2000);
    } catch (e: any) {
      set({ submitting: false });
      alert(e.response?.data?.error || 'Setup failed');
    }
  };

  return (
    <div className="min-h-screen flex items-center justify-center px-4"
         style={{ background: 'linear-gradient(135deg, var(--color-void) 0%, var(--color-cosmos) 100%)' }}>
      <div className="w-full max-w-lg">
        {/* Header */}
        <div className="text-center mb-8">
          <div className="flex items-center justify-center gap-3 mb-4">
            <BookOpen size={36} style={{ color: 'var(--color-nova-glow)' }} />
            <h1 style={{ fontFamily: 'var(--font-display)', fontSize: '2rem', color: 'var(--color-nova-glow)' }}>
              NovelWorld
            </h1>
          </div>
          <p style={{ color: 'var(--color-moonbeam)' }}>Welcome! Let's get you set up in 3 steps.</p>
        </div>

        {/* Progress */}
        <div className="flex items-center justify-center gap-2 mb-8">
          {[1, 2, 3].map(n => (
            <React.Fragment key={n}>
              <div className={`w-8 h-8 rounded-full flex items-center justify-center text-sm font-bold transition-all ${
                state.step >= n ? 'text-white' : ''
              }`} style={{
                background: state.step >= n
                  ? 'linear-gradient(135deg, var(--color-aurora), var(--color-nova))'
                  : 'rgba(15, 21, 53, 0.6)',
                border: '1px solid rgba(109, 40, 217, 0.3)',
                color: state.step >= n ? 'white' : 'var(--color-comet)',
              }}>
                {state.step > n ? <Check size={16} /> : n}
              </div>
              {n < 3 && <div className="w-12 h-0.5" style={{
                background: state.step > n ? 'var(--color-aurora)' : 'rgba(109, 40, 217, 0.2)',
              }} />}
            </React.Fragment>
          ))}
        </div>

        {/* Card */}
        <div className="rounded-xl p-8" style={{
          background: 'rgba(15, 21, 53, 0.8)',
          border: '1px solid rgba(109, 40, 217, 0.3)',
          backdropFilter: 'blur(20px)',
        }}>
          <AnimatePresence mode="wait">
            {/* Step 1: Choose Provider */}
            {state.step === 1 && (
              <motion.div key="step1" initial={{ opacity: 0, x: 20 }} animate={{ opacity: 1, x: 0 }} exit={{ opacity: 0, x: -20 }}>
                <div className="flex items-center gap-2 mb-6">
                  <Key size={20} style={{ color: 'var(--color-aurora-light)' }} />
                  <h2 className="text-lg font-semibold" style={{ color: 'var(--color-starlight)' }}>
                    Step 1: Connect AI Model
                  </h2>
                </div>

                <div className="grid grid-cols-2 gap-2 mb-4">
                  {PROVIDERS.map(p => (
                    <button
                      key={p.id}
                      onClick={() => set({ provider: p.id })}
                      className="p-3 rounded-lg text-left transition-all"
                      style={{
                        background: state.provider === p.id ? 'rgba(109, 40, 217, 0.3)' : 'rgba(3, 4, 10, 0.4)',
                        border: `1px solid ${state.provider === p.id ? 'rgba(109, 40, 217, 0.6)' : 'rgba(109, 40, 217, 0.15)'}`,
                      }}
                    >
                      <div className="text-sm font-medium" style={{ color: 'var(--color-starlight)' }}>{p.name}</div>
                      <div className="text-xs" style={{ color: 'var(--color-comet)' }}>{p.hint}</div>
                    </button>
                  ))}
                </div>

                {state.provider && (() => {
                  const selected = PROVIDERS.find(p => p.id === state.provider);
                  const isFree = selected?.free;
                  return (<>
                    {isFree ? (
                      <div className="p-3 rounded-lg mb-3 text-sm" style={{ background: 'rgba(34, 197, 94, 0.1)', border: '1px solid rgba(34, 197, 94, 0.3)', color: '#86efac' }}>
                        No API key needed. Make sure Ollama is running locally ({`ollama serve`}).
                      </div>
                    ) : (
                      <input
                        type="password"
                        value={state.apiKey}
                        onChange={e => set({ apiKey: e.target.value, testResult: 'idle' })}
                        placeholder={selected?.placeholder || 'API Key'}
                        className="w-full px-4 py-3 rounded-lg mb-3 outline-none"
                        style={{
                          background: 'rgba(3, 4, 10, 0.6)',
                          border: '1px solid rgba(109, 40, 217, 0.2)',
                          color: 'var(--color-starlight)',
                        }}
                      />
                    )}

                    <div className="flex gap-2">
                      <button
                        onClick={isFree ? () => set({ testResult: 'success' }) : testConnection}
                        disabled={!isFree && (!state.apiKey || state.testing)}
                        className="flex-1 py-2.5 rounded-lg font-medium transition-all flex items-center justify-center gap-2"
                        style={{
                          background: 'rgba(109, 40, 217, 0.3)',
                          border: '1px solid rgba(109, 40, 217, 0.4)',
                          color: 'var(--color-aurora-light)',
                          opacity: !isFree && !state.apiKey ? 0.5 : 1,
                        }}
                      >
                        {state.testing ? <Loader2 size={16} className="animate-spin" /> : null}
                        {state.testing ? 'Testing...' : isFree ? 'Continue' : 'Test Connection'}
                      </button>

                      {state.testResult === 'success' && (
                        <button
                          onClick={() => set({ step: 2 })}
                          className="px-6 py-2.5 rounded-lg font-semibold flex items-center gap-1"
                          style={{
                            background: 'linear-gradient(135deg, var(--color-aurora), var(--color-nova))',
                            color: 'white',
                          }}
                        >
                          Next <ChevronRight size={16} />
                        </button>
                      )}
                    </div>

                    {state.testResult === 'success' && (
                      <p className="mt-3 text-sm flex items-center gap-1" style={{ color: '#22c55e' }}>
                        <Check size={14} /> Connected successfully!
                      </p>
                    )}
                    {state.testResult === 'error' && (
                      <p className="mt-3 text-sm flex items-center gap-1" style={{ color: '#ef4444' }}>
                        <AlertCircle size={14} /> {state.testError}
                      </p>
                    )}
                  </>);
                })()}
              </motion.div>
            )}

            {/* Step 2: Create Admin Account */}
            {state.step === 2 && (
              <motion.div key="step2" initial={{ opacity: 0, x: 20 }} animate={{ opacity: 1, x: 0 }} exit={{ opacity: 0, x: -20 }}>
                <div className="flex items-center gap-2 mb-6">
                  <User size={20} style={{ color: 'var(--color-aurora-light)' }} />
                  <h2 className="text-lg font-semibold" style={{ color: 'var(--color-starlight)' }}>
                    Step 2: Create Your Account
                  </h2>
                </div>

                <div className="space-y-3">
                  <input
                    type="text"
                    value={state.name}
                    onChange={e => set({ name: e.target.value })}
                    placeholder="Your name (optional)"
                    className="w-full px-4 py-3 rounded-lg outline-none"
                    style={{
                      background: 'rgba(3, 4, 10, 0.6)',
                      border: '1px solid rgba(109, 40, 217, 0.2)',
                      color: 'var(--color-starlight)',
                    }}
                  />
                  <input
                    type="email"
                    value={state.email}
                    onChange={e => set({ email: e.target.value })}
                    placeholder="Email"
                    required
                    className="w-full px-4 py-3 rounded-lg outline-none"
                    style={{
                      background: 'rgba(3, 4, 10, 0.6)',
                      border: '1px solid rgba(109, 40, 217, 0.2)',
                      color: 'var(--color-starlight)',
                    }}
                  />
                  <input
                    type="password"
                    value={state.password}
                    onChange={e => set({ password: e.target.value })}
                    placeholder="Password (min 8 characters)"
                    minLength={8}
                    className="w-full px-4 py-3 rounded-lg outline-none"
                    style={{
                      background: 'rgba(3, 4, 10, 0.6)',
                      border: '1px solid rgba(109, 40, 217, 0.2)',
                      color: 'var(--color-starlight)',
                    }}
                  />
                </div>

                <div className="flex gap-2 mt-4">
                  <button
                    onClick={() => set({ step: 1 })}
                    className="px-4 py-2.5 rounded-lg"
                    style={{
                      background: 'rgba(3, 4, 10, 0.4)',
                      border: '1px solid rgba(109, 40, 217, 0.2)',
                      color: 'var(--color-moonbeam)',
                    }}
                  >
                    Back
                  </button>
                  <button
                    onClick={() => set({ step: 3 })}
                    disabled={!state.email || state.password.length < 8}
                    className="flex-1 py-2.5 rounded-lg font-semibold flex items-center justify-center gap-1"
                    style={{
                      background: 'linear-gradient(135deg, var(--color-aurora), var(--color-nova))',
                      color: 'white',
                      opacity: !state.email || state.password.length < 8 ? 0.5 : 1,
                    }}
                  >
                    Next <ChevronRight size={16} />
                  </button>
                </div>
              </motion.div>
            )}

            {/* Step 3: Confirm & Launch */}
            {state.step === 3 && (
              <motion.div key="step3" initial={{ opacity: 0, x: 20 }} animate={{ opacity: 1, x: 0 }} exit={{ opacity: 0, x: -20 }}>
                <div className="flex items-center gap-2 mb-6">
                  <Check size={20} style={{ color: 'var(--color-aurora-light)' }} />
                  <h2 className="text-lg font-semibold" style={{ color: 'var(--color-starlight)' }}>
                    Step 3: Ready to Launch
                  </h2>
                </div>

                <div className="space-y-3 mb-6">
                  <div className="flex justify-between p-3 rounded-lg" style={{ background: 'rgba(3, 4, 10, 0.4)' }}>
                    <span style={{ color: 'var(--color-moonbeam)' }}>AI Provider</span>
                    <span style={{ color: 'var(--color-starlight)' }}>
                      {PROVIDERS.find(p => p.id === state.provider)?.name}
                    </span>
                  </div>
                  <div className="flex justify-between p-3 rounded-lg" style={{ background: 'rgba(3, 4, 10, 0.4)' }}>
                    <span style={{ color: 'var(--color-moonbeam)' }}>Account</span>
                    <span style={{ color: 'var(--color-starlight)' }}>{state.email}</span>
                  </div>
                </div>

                <div className="flex gap-2">
                  <button
                    onClick={() => set({ step: 2 })}
                    className="px-4 py-2.5 rounded-lg"
                    style={{
                      background: 'rgba(3, 4, 10, 0.4)',
                      border: '1px solid rgba(109, 40, 217, 0.2)',
                      color: 'var(--color-moonbeam)',
                    }}
                  >
                    Back
                  </button>
                  <button
                    onClick={finishSetup}
                    disabled={state.submitting}
                    className="flex-1 py-3 rounded-lg font-semibold flex items-center justify-center gap-2"
                    style={{
                      background: 'linear-gradient(135deg, var(--color-aurora), var(--color-nova))',
                      color: 'white',
                      opacity: state.submitting ? 0.7 : 1,
                    }}
                  >
                    {state.submitting ? <Loader2 size={16} className="animate-spin" /> : <BookOpen size={16} />}
                    {state.submitting ? 'Setting up...' : 'Launch NovelWorld'}
                  </button>
                </div>
              </motion.div>
            )}

            {/* Step 4: Success */}
            {state.step === 4 && (
              <motion.div key="step4" initial={{ opacity: 0, scale: 0.9 }} animate={{ opacity: 1, scale: 1 }}
                          className="text-center py-8">
                <div className="w-16 h-16 rounded-full mx-auto mb-4 flex items-center justify-center"
                     style={{ background: 'rgba(34, 197, 94, 0.2)' }}>
                  <Check size={32} style={{ color: '#22c55e' }} />
                </div>
                <h2 className="text-xl font-semibold mb-2" style={{ color: 'var(--color-starlight)' }}>
                  All Set!
                </h2>
                <p style={{ color: 'var(--color-moonbeam)' }}>
                  Entering NovelWorld...
                </p>
                <Loader2 size={20} className="animate-spin mx-auto mt-4" style={{ color: 'var(--color-nova-glow)' }} />
              </motion.div>
            )}
          </AnimatePresence>
        </div>
      </div>
    </div>
  );
}
