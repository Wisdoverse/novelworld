import { useEffect, useState, type FormEvent } from 'react';
import type { CreatePlayerEntityInput } from '@/entities/narrative/api';

interface PlayerEntryFormProps {
  checkpointChapter: number;
  unlockedThroughChapter: number;
  locations: Array<{ id: string; name: string }>;
  isPending: boolean;
  error?: string;
  onCheckpointChange: (chapter: number) => void;
  onSubmit: (input: CreatePlayerEntityInput) => Promise<unknown>;
}

function tokens(value: string) {
  return value.split(/[,，]/).map(token => token.trim()).filter(Boolean);
}

export function PlayerEntryForm({
  checkpointChapter,
  unlockedThroughChapter,
  locations,
  isPending,
  error,
  onCheckpointChange,
  onSubmit,
}: PlayerEntryFormProps) {
  const [name, setName] = useState('');
  const [background, setBackground] = useState('');
  const [capabilities, setCapabilities] = useState('');
  const [locationId, setLocationId] = useState(locations[0]?.id ?? '');
  const [inventory, setInventory] = useState('');

  useEffect(() => {
    if (!locations.some(location => location.id === locationId)) {
      setLocationId(locations[0]?.id ?? '');
    }
  }, [locationId, locations]);

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!locationId) return;
    try {
      await onSubmit({
        checkpoint_chapter: checkpointChapter,
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
      className="surface-card mt-8 p-6"
      aria-labelledby="player-entry-title"
    >
      <h2 id="player-entry-title" className="text-xl font-semibold text-[#1f1f1f]">
        创建你的原创角色
      </h2>
      <p className="mt-2 text-sm text-[#5f6368]">
        从已解锁章节选择入场点；后续原著事件会从该处继续运行。
      </p>
      <form className="mt-5 space-y-4" onSubmit={submit}>
        <label className="block text-sm font-medium text-[#3c4043]">
          入场章节
          <select
            className="field-control mt-1"
            value={checkpointChapter}
            onChange={event => onCheckpointChange(Number(event.target.value))}
          >
            {Array.from({ length: unlockedThroughChapter }, (_, index) => index + 1).map(chapter => (
              <option key={chapter} value={chapter}>第 {chapter} 章</option>
            ))}
          </select>
        </label>
        <label className="block text-sm font-medium text-[#3c4043]">
          名字
          <input
            className="field-control mt-1"
            value={name}
            onChange={event => setName(event.target.value)}
            maxLength={100}
            required
          />
        </label>
        <label className="block text-sm font-medium text-[#3c4043]">
          背景
          <textarea
            className="field-control mt-1"
            value={background}
            onChange={event => setBackground(event.target.value)}
            maxLength={2000}
            rows={3}
            required
          />
        </label>
        <label className="block text-sm font-medium text-[#3c4043]">
          能力（用逗号分隔）
          <input
            className="field-control mt-1"
            value={capabilities}
            onChange={event => setCapabilities(event.target.value)}
            maxLength={3200}
            required
          />
        </label>
        <label className="block text-sm font-medium text-[#3c4043]">
          初始地点
          <select
            className="field-control mt-1"
            value={locationId}
            onChange={event => setLocationId(event.target.value)}
            required
          >
            {locations.map(location => (
              <option key={location.id} value={location.id}>{location.name}</option>
            ))}
          </select>
        </label>
        <label className="block text-sm font-medium text-[#3c4043]">
          随身物品（可选，用逗号分隔）
          <input
            className="field-control mt-1"
            value={inventory}
            onChange={event => setInventory(event.target.value)}
            maxLength={6400}
          />
        </label>
        {locations.length === 0 ? <p role="alert" className="text-sm text-[#b3261e]">当前进度没有可用地点。</p> : null}
        {error ? <p role="alert" className="text-sm text-[#b3261e]">{error}</p> : null}
        <button
          type="submit"
          disabled={isPending || locations.length === 0}
          className="primary-action"
        >
          {isPending ? '正在进入世界…' : '进入故事'}
        </button>
      </form>
    </section>
  );
}
