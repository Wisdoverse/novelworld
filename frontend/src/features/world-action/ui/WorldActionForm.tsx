import { useEffect, useMemo, useRef, useState, type FormEvent } from 'react';
import { suggestWorldAction } from '@/entities/narrative';
import type { OpenWorldView, WorldAction, WorldActionKind } from '@/shared/types';

interface WorldActionFormProps {
  view: OpenWorldView;
  isPending: boolean;
  isLocked?: boolean;
  onSubmit: (action: WorldAction) => Promise<unknown>;
}

export const actionLabels: Record<WorldActionKind, string> = {
  travel: '前往地点',
  investigate: '调查线索',
  converse: '与角色交谈',
  ally: '争取结盟',
  oppose: '公开反对',
  advance_thread: '推进事件线',
  resolve_thread: '解决事件线（旧版）',
  pursue_goal: '追求自己的目标',
};

const availableActions: WorldActionKind[] = [
  'travel',
  'investigate',
  'converse',
  'ally',
  'oppose',
  'advance_thread',
  'pursue_goal',
];

function targets(view: OpenWorldView, kind: WorldActionKind) {
  const { entry_context: context } = view.session;
  if (kind === 'travel') return context.locations;
  if (kind === 'converse' || kind === 'ally' || kind === 'oppose') {
    return context.characters.filter(character => (
      !view.session.dead_character_ids.includes(character.id)
    ));
  }
  if (kind === 'advance_thread' || kind === 'resolve_thread') {
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
  const [targetId, setTargetId] = useState<string | null>('');
  const [intent, setIntent] = useState('');
  const [suggesting, setSuggesting] = useState(false);
  const [suggestion, setSuggestion] = useState<{ kind: WorldActionKind | null; view: OpenWorldView } | null>(null);
  const suggestionRequest = useRef<AbortController | null>(null);
  const targetOptions = useMemo(() => targets(view, kind), [kind, view]);
  const targetRequired = kind !== 'pursue_goal';
  const controlsDisabled = isPending || isLocked;
  const latest = useRef({ view, intent, kind, controlsDisabled });
  latest.current = { view, intent, kind, controlsDisabled };
  useEffect(() => {
    suggestionRequest.current?.abort();
    suggestionRequest.current = null;
    setSuggestion(null);
    setSuggesting(false);
  }, [view, controlsDisabled]);
  useEffect(() => () => suggestionRequest.current?.abort(), []);

  const clearSuggestion = () => {
    suggestionRequest.current?.abort();
    suggestionRequest.current = null;
    setSuggestion(null);
    setSuggesting(false);
  };

  const requestSuggestion = async () => {
    if (controlsDisabled || !view.action_suggestions_available || !intent.trim()) return;
    const controller = new AbortController();
    const requestedIntent = intent.trim();
    const requestedKind = kind;
    suggestionRequest.current = controller;
    setSuggestion(null);
    setSuggesting(true);
    try {
      const result = await suggestWorldAction(view.player.novel_id, requestedIntent, controller.signal);
      const current = latest.current;
      if (suggestionRequest.current === controller && current.view === view
        && current.intent.trim() === requestedIntent && current.kind === requestedKind
        && !current.controlsDisabled) {
        setSuggestion({ kind: result, view });
      }
    } catch {
      if (suggestionRequest.current === controller && !controller.signal.aborted) {
        setSuggestion({ kind: null, view });
      }
    } finally {
      if (suggestionRequest.current === controller) {
        suggestionRequest.current = null;
        setSuggesting(false);
      }
    }
  };
  const selectedTarget = targetId === null ? '' : targetOptions.some(option => option.id === targetId)
    ? targetId
    : targetRequired ? targetOptions[0]?.id ?? '' : '';
  const actionRule = view.session.game_rules?.action_rules.find(rule => rule.kind === kind);
  const actionAttribute = view.session.game_rules?.attributes.find(
    attribute => attribute.key === actionRule?.attribute_key,
  );
  const actionScore = actionAttribute
    ? view.player.rules?.attributes[actionAttribute.key]
    : undefined;
  const actionModifier = actionScore === undefined ? undefined : Math.floor((actionScore - 10) / 2);

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (controlsDisabled || (targetRequired && !selectedTarget)) return;
    clearSuggestion();
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
      <p className="text-sm text-[#5f6368]">
        行动者始终是你创建的角色“{view.player.name}”；原著角色会依据自己的目标回应。
      </p>
      <label className="block text-sm font-medium text-[#3c4043]">
        行动
        <select
          className="field-control mt-1"
          disabled={controlsDisabled}
          value={kind}
          onChange={event => {
            clearSuggestion();
            setKind(event.target.value as WorldActionKind);
            setTargetId('');
          }}
        >
          {availableActions.map(value => (
            <option key={value} value={value}>{actionLabels[value]}</option>
          ))}
        </select>
      </label>
      {actionRule && actionAttribute && actionScore !== undefined && actionModifier !== undefined ? (
        <div className="rounded-lg border border-[#d2e3fc] bg-[#f8faff] p-3 text-sm text-[#3c4043]">
          <span className="font-semibold text-[#0b57d0]">检定预览</span>
          <span className="ml-2">D20 + {actionAttribute.label} {actionModifier >= 0 ? `+${actionModifier}` : actionModifier}，模板基础难度 {actionRule.difficulty_class}</span>
          <p className="mt-1 text-xs text-[#5f6368]">
            {actionRule.description}；提交后服务端会进行语义判断，可能无需检定或调整难度；配置缺失时沿用模板检定。实际结果由服务端保存并可回放。
          </p>
        </div>
      ) : null}
      <label className="block text-sm font-medium text-[#3c4043]">
        目标{targetRequired ? '' : '（可选）'}
        <select
          className="field-control mt-1"
          disabled={controlsDisabled}
          value={selectedTarget}
          onChange={event => {
            clearSuggestion();
            setTargetId(event.target.value);
          }}
          required={targetRequired}
        >
          {!targetRequired ? <option value="">自定目标</option> : null}
          {targetRequired && targetId === null ? <option value="" disabled>请选择目标</option> : null}
          {targetOptions.map(option => (
            <option key={option.id} value={option.id}>{option.name}</option>
          ))}
        </select>
      </label>
      {targetRequired && targetOptions.length === 0 ? (
        <p role="alert" className="text-sm text-[#b3261e]">
          当前世界状态没有适合此行动的目标。
        </p>
      ) : null}
      <label className="block text-sm font-medium text-[#3c4043]">
        你的意图
        <textarea
          className="field-control mt-1"
          disabled={controlsDisabled}
          value={intent}
          onChange={event => {
            clearSuggestion();
            setIntent(event.target.value);
          }}
          maxLength={500}
          rows={3}
          required
        />
      </label>
      {view.action_suggestions_available ? (
        <div className="text-sm text-[#3c4043]">
          <button
            type="button"
            className="underline disabled:opacity-50"
            disabled={controlsDisabled || suggesting || !intent.trim()}
            onClick={() => void requestSuggestion()}
          >
            {suggesting ? '正在分析行动…' : '建议行动类型'}
          </button>
          <p className="mt-1 text-xs text-[#5f6368]">
            点击后会将当前意图发送至部署方配置的 Laya 服务。
          </p>
          {suggestion?.view === view && !controlsDisabled ? (
            <div role="status" className="mt-1">
              {suggestion.kind ? (
                <>
                  建议：
                  <button
                    type="button"
                    className="underline"
                    onClick={() => {
                      setKind(suggestion.kind!);
                      setTargetId(null);
                      clearSuggestion();
                    }}
                  >
                    {actionLabels[suggestion.kind]}
                  </button>
                  。点击可选用行动类型，再确认目标并提交。
                </>
              ) : '暂时没有可靠建议，请自行选择行动。'}
            </div>
          ) : null}
        </div>
      ) : null}
      <button
        type="submit"
        disabled={controlsDisabled || !intent.trim() || (targetRequired && !selectedTarget)}
        className="primary-action"
      >
        {isPending ? '世界正在回应…' : '执行行动'}
      </button>
    </form>
  );
}
