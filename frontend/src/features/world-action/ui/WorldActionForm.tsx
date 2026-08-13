import { useMemo, useState, type FormEvent } from 'react';
import type { OpenWorldView, WorldAction, WorldActionKind } from '@/shared/types';

interface WorldActionFormProps {
  view: OpenWorldView;
  isPending: boolean;
  isLocked?: boolean;
  onSubmit: (action: WorldAction) => Promise<unknown>;
}

const actionLabels: Record<WorldActionKind, string> = {
  travel: '前往地点',
  investigate: '调查线索',
  converse: '与角色交谈',
  ally: '争取结盟',
  oppose: '公开反对',
  resolve_thread: '解决事件线',
  pursue_goal: '追求自己的目标',
};

function targets(view: OpenWorldView, kind: WorldActionKind) {
  const { entry_context: context } = view.session;
  if (kind === 'travel') return context.locations;
  if (kind === 'converse' || kind === 'ally' || kind === 'oppose') {
    return context.characters.filter(character => (
      !view.session.dead_character_ids.includes(character.id)
    ));
  }
  if (kind === 'resolve_thread') {
    return Object.entries(view.world_state.state.threads ?? {})
      .filter(([, thread]) => thread.status === 'open')
      .map(([id, thread]) => ({ id, name: thread.description }));
  }
  if (kind === 'investigate') {
    const threads = Object.entries(view.world_state.state.threads ?? {})
      .filter(([, thread]) => thread.status === 'open')
      .map(([id, thread]) => ({ id, name: `事件线：${thread.description}` }));
    const events = view.session.canonical_events
      .filter(event => event.status === 'scheduled' || event.status === 'delayed')
      .map(event => ({ id: event.id, name: `主线事件：${event.summary}` }));
    return [...context.locations, ...threads, ...events];
  }
  return context.character_goals.map(goal => ({ id: goal.id, name: goal.description }));
}

export function WorldActionForm({ view, isPending, isLocked = false, onSubmit }: WorldActionFormProps) {
  const [kind, setKind] = useState<WorldActionKind>('travel');
  const [targetId, setTargetId] = useState('');
  const [intent, setIntent] = useState('');
  const targetOptions = useMemo(() => targets(view, kind), [kind, view]);
  const targetRequired = kind !== 'pursue_goal';
  const selectedTarget = targetOptions.some(option => option.id === targetId)
    ? targetId
    : targetRequired ? targetOptions[0]?.id ?? '' : '';

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (targetRequired && !selectedTarget) return;
    try {
      await onSubmit({
        kind,
        target_id: selectedTarget || null,
        intent: intent.trim(),
      });
      setIntent('');
    } catch {
      // The dashboard retains the exact request and renders its retry control.
    }
  };

  return (
    <form className="space-y-4" onSubmit={submit}>
      <p className="text-sm" style={{ color: '#94a3b8' }}>
        行动者始终是你创建的角色“{view.player.name}”；原著角色会依据自己的目标回应。
      </p>
      <label className="block text-sm" style={{ color: '#cbd5e1' }}>
        行动
        <select
          className="mt-1 w-full rounded-lg px-3 py-2"
          style={{ background: 'rgba(15, 23, 42, 0.8)', border: '1px solid #334155' }}
          value={kind}
          onChange={event => {
            setKind(event.target.value as WorldActionKind);
            setTargetId('');
          }}
        >
          {Object.entries(actionLabels).map(([value, label]) => (
            <option key={value} value={value}>{label}</option>
          ))}
        </select>
      </label>
      <label className="block text-sm" style={{ color: '#cbd5e1' }}>
        目标{targetRequired ? '' : '（可选）'}
        <select
          className="mt-1 w-full rounded-lg px-3 py-2"
          style={{ background: 'rgba(15, 23, 42, 0.8)', border: '1px solid #334155' }}
          value={selectedTarget}
          onChange={event => setTargetId(event.target.value)}
          required={targetRequired}
        >
          {!targetRequired ? <option value="">自定目标</option> : null}
          {targetOptions.map(option => (
            <option key={option.id} value={option.id}>{option.name}</option>
          ))}
        </select>
      </label>
      {targetRequired && targetOptions.length === 0 ? (
        <p role="alert" className="text-sm" style={{ color: '#fca5a5' }}>
          当前世界状态没有适合此行动的目标。
        </p>
      ) : null}
      <label className="block text-sm" style={{ color: '#cbd5e1' }}>
        你的意图
        <textarea
          className="mt-1 w-full rounded-lg px-3 py-2"
          style={{ background: 'rgba(15, 23, 42, 0.8)', border: '1px solid #334155' }}
          value={intent}
          onChange={event => setIntent(event.target.value)}
          maxLength={500}
          rows={3}
          required
        />
      </label>
      <button
        type="submit"
        disabled={isPending || isLocked || !intent.trim() || (targetRequired && !selectedTarget)}
        className="px-4 py-2 rounded-lg text-sm font-medium disabled:opacity-50"
        style={{ background: '#0891b2', color: 'white' }}
      >
        {isPending ? '世界正在回应…' : '执行行动'}
      </button>
    </form>
  );
}
