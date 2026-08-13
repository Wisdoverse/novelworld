import { useState, type FormEvent } from 'react';
import type { CreatePlayerEntityInput } from '@/entities/narrative/api';

interface PlayerEntryFormProps {
  checkpointChapter: number;
  locations: Array<{ id: string; name: string }>;
  isPending: boolean;
  error?: string;
  onSubmit: (input: CreatePlayerEntityInput) => Promise<unknown>;
}

function tokens(value: string) {
  return value.split(/[,，]/).map(token => token.trim()).filter(Boolean);
}

export function PlayerEntryForm({
  checkpointChapter,
  locations,
  isPending,
  error,
  onSubmit,
}: PlayerEntryFormProps) {
  const [name, setName] = useState('');
  const [background, setBackground] = useState('');
  const [capabilities, setCapabilities] = useState('');
  const [locationId, setLocationId] = useState(locations[0]?.id ?? '');
  const [inventory, setInventory] = useState('');

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!locationId) return;
    try {
      await onSubmit({
        name: name.trim(),
        background: background.trim(),
        capabilities: tokens(capabilities),
        location_id: locationId,
        inventory: tokens(inventory),
      });
    } catch {
      // The mutation error is rendered by the parent.
    }
  };

  return (
    <section
      className="mt-8 p-6 rounded-xl"
      style={{ background: 'rgba(109, 40, 217, 0.08)', border: '1px solid rgba(109, 40, 217, 0.25)' }}
      aria-labelledby="player-entry-title"
    >
      <h2 id="player-entry-title" className="text-xl font-semibold" style={{ color: '#e2e8f0' }}>
        创建你的原创角色
      </h2>
      <p className="mt-2 text-sm" style={{ color: '#94a3b8' }}>
        角色将从你已读到的第 {checkpointChapter} 章进入故事；可选地点不会剧透后续内容。
      </p>
      <form className="mt-5 space-y-4" onSubmit={submit}>
        <label className="block text-sm" style={{ color: '#cbd5e1' }}>
          名字
          <input
            className="mt-1 w-full rounded-lg px-3 py-2"
            style={{ background: 'rgba(15, 23, 42, 0.8)', border: '1px solid #334155' }}
            value={name}
            onChange={event => setName(event.target.value)}
            maxLength={100}
            required
          />
        </label>
        <label className="block text-sm" style={{ color: '#cbd5e1' }}>
          背景
          <textarea
            className="mt-1 w-full rounded-lg px-3 py-2"
            style={{ background: 'rgba(15, 23, 42, 0.8)', border: '1px solid #334155' }}
            value={background}
            onChange={event => setBackground(event.target.value)}
            maxLength={2000}
            rows={3}
            required
          />
        </label>
        <label className="block text-sm" style={{ color: '#cbd5e1' }}>
          能力（用逗号分隔）
          <input
            className="mt-1 w-full rounded-lg px-3 py-2"
            style={{ background: 'rgba(15, 23, 42, 0.8)', border: '1px solid #334155' }}
            value={capabilities}
            onChange={event => setCapabilities(event.target.value)}
            maxLength={3200}
            required
          />
        </label>
        <label className="block text-sm" style={{ color: '#cbd5e1' }}>
          初始地点
          <select
            className="mt-1 w-full rounded-lg px-3 py-2"
            style={{ background: 'rgba(15, 23, 42, 0.8)', border: '1px solid #334155' }}
            value={locationId}
            onChange={event => setLocationId(event.target.value)}
            required
          >
            {locations.map(location => (
              <option key={location.id} value={location.id}>{location.name}</option>
            ))}
          </select>
        </label>
        <label className="block text-sm" style={{ color: '#cbd5e1' }}>
          随身物品（可选，用逗号分隔）
          <input
            className="mt-1 w-full rounded-lg px-3 py-2"
            style={{ background: 'rgba(15, 23, 42, 0.8)', border: '1px solid #334155' }}
            value={inventory}
            onChange={event => setInventory(event.target.value)}
            maxLength={6400}
          />
        </label>
        {locations.length === 0 ? <p role="alert" className="text-sm" style={{ color: '#fca5a5' }}>当前进度没有可用地点。</p> : null}
        {error ? <p role="alert" className="text-sm" style={{ color: '#fca5a5' }}>{error}</p> : null}
        <button
          type="submit"
          disabled={isPending || locations.length === 0}
          className="px-4 py-2 rounded-lg text-sm font-medium disabled:opacity-50"
          style={{ background: '#6d28d9', color: 'white' }}
        >
          {isPending ? '正在进入世界…' : '进入故事'}
        </button>
      </form>
    </section>
  );
}
