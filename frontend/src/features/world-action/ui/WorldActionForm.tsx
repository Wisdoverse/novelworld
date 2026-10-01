import { useEffect, useId, useMemo, useRef, useState, type FormEvent } from 'react';
import { suggestWorldAction } from '@/entities/narrative';
import { effectiveWorldContext } from '@/shared/lib/worldSourceContext';
import { localWorldCharacterIds } from '@/shared/lib/localWorldCharacters';
import type { OpenWorldView, WorldAction, WorldActionKind } from '@/shared/types';

interface WorldActionFormProps {
  view: OpenWorldView;
  isPending: boolean;
  isLocked?: boolean;
  onSubmit: (action: WorldAction) => Promise<unknown>;
}

interface Target { id: string; name: string }
interface SceneSuggestion extends WorldAction { label: string; key: string }

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
  'travel', 'investigate', 'converse', 'ally', 'oppose', 'advance_thread', 'pursue_goal',
];

function boundedIntent(value: string) {
  return Array.from(value.trim()).slice(0, 500).join('');
}

function targets(view: OpenWorldView, kind: WorldActionKind): Target[] {
  const context = effectiveWorldContext(view.session);
  if (kind === 'travel') return context.locations;
  if (kind === 'converse' || kind === 'ally' || kind === 'oppose') {
    const local = localWorldCharacterIds(view);
    return context.characters.filter(character => local.has(character.id));
  }

  const allowedThreadIds = new Set(context.threads.map(thread => thread.id));
  const threads = Object.entries(view.world_state.state.threads ?? {})
    .filter(([id, thread]) => allowedThreadIds.has(id) && thread.status === 'open')
    .map(([id, thread]) => ({ id, name: thread.description }));
  if (kind === 'advance_thread' || kind === 'resolve_thread') return threads;

  if (kind === 'investigate') {
    const admittedEvents = new Map(context.scheduled_events.map(event => [event.id, event]));
    const highWater = context.unlocked_through_chapter;
    const events = view.session.canonical_events
      .filter(event => {
        const admitted = admittedEvents.get(event.id);
        return (event.status === 'scheduled' || event.status === 'delayed')
          && admitted !== undefined
          && event.source_chapters.length > 0
          && event.source_chapters.every(chapter => chapter <= highWater)
          && admitted.source_chapters.length > 0
          && admitted.source_chapters.every(chapter => chapter <= highWater);
      })
      .map(event => ({ id: event.id, name: `主线事件：${event.summary}` }));
    return [...context.locations, ...threads.map(thread => ({ ...thread, name: `事件线：${thread.name}` })), ...events];
  }

  return context.character_goals.map(goal => ({ id: goal.id, name: goal.description }));
}

function sceneFingerprint(view: OpenWorldView) {
  const context = effectiveWorldContext(view.session);
  const admittedEventIds = new Set(targets(view, 'investigate')
    .filter(target => target.name.startsWith('主线事件：'))
    .map(target => target.id));
  return JSON.stringify([
    view.session.turn_number,
    context.checkpoint_chapter,
    context.unlocked_through_chapter,
    view.player.location_id,
    targets(view, 'converse'),
    targets(view, 'investigate'),
    targets(view, 'travel'),
    targets(view, 'advance_thread'),
    targets(view, 'pursue_goal'),
    view.session.canonical_events
      .filter(event => admittedEventIds.has(event.id))
      .map(event => [event.id, event.status, event.source_chapters]),
  ]);
}

function sceneSuggestions(view: OpenWorldView): SceneSuggestion[] {
  const localCharacter = targets(view, 'converse')[0];
  const investigateTargets = targets(view, 'investigate');
  const currentLocation = view.player.location_id;
  const openThread = investigateTargets.find(target => target.name.startsWith('事件线：'));
  const canonicalEvent = investigateTargets.find(target => target.name.startsWith('主线事件：'));
  const currentPlace = investigateTargets.find(target => target.id === currentLocation);
  const investigateTarget = openThread ?? canonicalEvent ?? currentPlace ?? investigateTargets[0];
  const investigateSubject = investigateTarget?.name.replace(/^(事件线|主线事件)：/, '');
  const nextLocation = targets(view, 'travel').find(location => location.id !== currentLocation);
  const fallback = [
    { label: '留意周围', intent: '留意当前周围，看看有什么值得关注。' },
    { label: '整理线索', intent: '整理目前掌握的线索，想好下一步行动。' },
    { label: '计划下一步', intent: '根据目前的情况，想好接下来要做什么。' },
  ];
  const suggestions: SceneSuggestion[] = [];
  const add = (kind: WorldActionKind, target: Target | undefined, label: string, intent: string) => {
    suggestions.push({
      kind: target ? kind : 'pursue_goal',
      target_id: target?.id ?? null,
      label: `${suggestions.length + 1}. ${target ? label : fallback[suggestions.length].label}`,
      intent: boundedIntent(target ? intent : fallback[suggestions.length].intent),
      key: `${target ? kind : 'pursue_goal'}:${target?.id ?? 'self'}:${suggestions.length}`,
    });
  };

  add('converse', localCharacter, `与${localCharacter?.name ?? ''}交谈`, `尝试与${localCharacter?.name ?? ''}交谈，了解当前局势。`);
  add('investigate', investigateTarget, `留意${investigateSubject ?? ''}`, `看看${investigateSubject ?? '当前周围'}的情况，想好下一步。`);
  add('travel', nextLocation, `前往${nextLocation?.name ?? ''}`, `前往${nextLocation?.name ?? '别处'}，看看那里的情况。`);
  return suggestions;
}

export function WorldActionForm({ view, isPending, isLocked = false, onSubmit }: WorldActionFormProps) {
  const fingerprint = useMemo(() => sceneFingerprint(view), [view]);
  const suggestions = useMemo(() => sceneSuggestions(view), [view]);
  const [kind, setKind] = useState<WorldActionKind | ''>('pursue_goal');
  const [targetId, setTargetId] = useState<string | null>(null);
  const [intent, setIntent] = useState('');
  const [draftScene, setDraftScene] = useState(fingerprint);
  const [selectedSuggestion, setSelectedSuggestion] = useState<SceneSuggestion | null>(null);
  const [suggesting, setSuggesting] = useState(false);
  const [classifier, setClassifier] = useState<{ kind: WorldActionKind | null; fingerprint: string } | null>(null);
  const suggestionRequest = useRef<AbortController | null>(null);
  const seenScene = useRef(fingerprint);
  const controlsDisabled = isPending || isLocked;
  const latest = useRef({ view, fingerprint, intent, kind, controlsDisabled, selectedSuggestion, targetId });
  latest.current = { view, fingerprint, intent, kind, controlsDisabled, selectedSuggestion, targetId };
  const sceneChanged = Boolean(intent.trim()) && draftScene !== fingerprint;
  const targetOptions = useMemo(() => kind ? targets(view, kind) : [], [kind, view]);
  const selectedTarget = targetId && targetOptions.some(option => option.id === targetId) ? targetId : '';
  const targetRequired = kind !== '' && kind !== 'pursue_goal';
  const guidanceId = useId();

  useEffect(() => {
    suggestionRequest.current?.abort();
    suggestionRequest.current = null;
    setClassifier(null);
    setSuggesting(false);
    if (seenScene.current !== fingerprint) {
      seenScene.current = fingerprint;
      if (latest.current.selectedSuggestion) {
        const untouched = latest.current.intent === latest.current.selectedSuggestion.intent;
        if (untouched) setIntent('');
        setKind('pursue_goal');
        setTargetId(null);
        if (untouched) setDraftScene(fingerprint);
        setSelectedSuggestion(null);
      } else {
        if (!latest.current.intent.trim()) setDraftScene(fingerprint);
        if (latest.current.targetId && latest.current.kind
          && !targets(latest.current.view, latest.current.kind).some(option => option.id === latest.current.targetId)) {
          setTargetId(latest.current.kind === 'pursue_goal' ? null : '');
        }
      }
    }
  }, [controlsDisabled, fingerprint]);
  useEffect(() => () => suggestionRequest.current?.abort(), []);

  const clearClassifier = () => {
    suggestionRequest.current?.abort();
    suggestionRequest.current = null;
    setClassifier(null);
    setSuggesting(false);
  };

  const requestSuggestion = async () => {
    if (controlsDisabled || !view.action_suggestions_available || !intent.trim()) return;
    const controller = new AbortController();
    const requestedIntent = intent.trim();
    const requestedKind = kind;
    suggestionRequest.current = controller;
    setClassifier(null);
    setSuggesting(true);
    try {
      const result = await suggestWorldAction(view.player.novel_id, requestedIntent, controller.signal);
      const current = latest.current;
      if (suggestionRequest.current === controller && current.fingerprint === fingerprint
        && current.intent.trim() === requestedIntent && current.kind === requestedKind
        && !current.controlsDisabled) {
        setClassifier({ kind: result, fingerprint });
      }
    } catch {
      if (suggestionRequest.current === controller && !controller.signal.aborted) {
        setClassifier({ kind: null, fingerprint });
      }
    } finally {
      if (suggestionRequest.current === controller) {
        suggestionRequest.current = null;
        setSuggesting(false);
      }
    }
  };

  const blockingReason = controlsDisabled ? undefined
    : sceneChanged ? '场景已变化，请重新选择建议，或确认这份草稿仍适合当前场景。'
      : !kind ? '请选择场景建议、自由输入或调整行动方式。'
        : targetRequired && targetOptions.length === 0
          ? '当前没有可供此行动选择的目标。可以选择自由输入，写下你的下一步行动。'
          : targetRequired && !selectedTarget ? '请先选择这次行动的目标。'
            : !intent.trim() ? '请在“你的意图”中写下你想做什么。' : undefined;
  const actionRule = view.session.game_rules?.action_rules.find(rule => rule.kind === kind);
  const actionAttribute = view.session.game_rules?.attributes.find(attribute => attribute.key === actionRule?.attribute_key);
  const actionScore = actionAttribute ? view.player.rules?.attributes[actionAttribute.key] : undefined;
  const actionModifier = actionScore === undefined ? undefined : Math.floor((actionScore - 10) / 2);

  const chooseSuggestion = (choice: SceneSuggestion) => {
    clearClassifier();
    setKind(choice.kind);
    setTargetId(choice.target_id);
    setIntent(choice.intent);
    setDraftScene(fingerprint);
    setSelectedSuggestion(choice);
  };

  const chooseFreeInput = () => {
    clearClassifier();
    setKind('pursue_goal');
    setTargetId(null);
    setDraftScene(fingerprint);
    setSelectedSuggestion(null);
  };

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (controlsDisabled || blockingReason || !kind) return;
    const currentTargets = targets(view, kind);
    const validTarget = targetId && currentTargets.some(option => option.id === targetId) ? targetId : null;
    if (kind !== 'pursue_goal' && !validTarget) return;
    clearClassifier();
    try {
      await onSubmit({ kind, target_id: validTarget, intent: intent.trim() });
      setIntent('');
      setKind('pursue_goal');
      setTargetId(null);
      setDraftScene(latest.current.fingerprint);
      setSelectedSuggestion(null);
    } catch {
      // The dashboard retains the exact request and renders its retry control.
    }
  };

  return (
    <form className="space-y-4" onSubmit={submit}>
      <p className="text-sm text-[#5f6368]">
        行动者始终是你创建的角色“{view.player.name}”；原著角色会依据自己的目标回应。
      </p>
      <fieldset className="space-y-2">
        <legend className="mb-2 text-sm font-medium text-[#3c4043]">场景建议</legend>
        <div className="flex flex-wrap gap-2">
          {suggestions.map((choice, index) => (
            <button
              key={choice.key}
              type="button"
              disabled={controlsDisabled}
              aria-label={choice.label}
              aria-pressed={selectedSuggestion?.key === choice.key && !sceneChanged}
              className={`min-h-11 min-w-0 max-w-full break-words [overflow-wrap:anywhere] rounded-lg border px-3 py-2 text-left text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#315b45] focus-visible:ring-offset-2 disabled:opacity-50 ${selectedSuggestion?.key === choice.key && !sceneChanged ? 'border-[#315b45] bg-[#dce8d3] text-[#203a35] ring-1 ring-[#315b45]' : 'border-[#c8d8c0] bg-[#f4f7ef] text-[#203a35] hover:bg-[#e8efe2]'}`}
              onClick={() => chooseSuggestion(choice)}
            >
              <span className="mr-2 font-semibold text-[#315b45]">{index + 1}.</span>{choice.label.slice(3)}
            </button>
          ))}
        </div>
      </fieldset>
      <button
        type="button"
        disabled={controlsDisabled}
        aria-pressed={kind === 'pursue_goal' && !selectedSuggestion && !sceneChanged}
        className={`min-h-11 max-w-full break-words [overflow-wrap:anywhere] rounded-lg border px-3 py-2 text-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#315b45] focus-visible:ring-offset-2 disabled:opacity-50 ${kind === 'pursue_goal' && !selectedSuggestion && !sceneChanged ? 'border-[#315b45] bg-[#dce8d3] text-[#203a35] ring-1 ring-[#315b45]' : 'border-[#c8d8c0] bg-white text-[#203a35] hover:bg-[#f4f7ef]'}`}
        onClick={chooseFreeInput}
      >
        自由输入
      </button>
      <label className="block text-sm font-medium text-[#3c4043]">
        你的意图
        <textarea
          className="field-control mt-1"
          disabled={controlsDisabled}
          value={intent}
          onChange={event => {
            clearClassifier();
            setIntent(event.target.value);
          }}
          maxLength={500}
          rows={3}
          required
        />
      </label>
      {sceneChanged && !controlsDisabled ? (
        <div role="status" className="text-sm text-[#8a4b08]">
          场景已变化，请重新选择建议或确认这份草稿。
          <button type="button" className="ml-2 min-h-11 rounded px-2 font-medium underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#315b45]" onClick={() => setDraftScene(fingerprint)}>
            确认在当前场景继续
          </button>
        </div>
      ) : null}
      <details className="rounded-lg border border-[#ded6c4] bg-white/70 p-3">
        <summary className="cursor-pointer rounded text-sm font-medium text-[#315b45] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#315b45]">调整行动方式与目标</summary>
        <div className="mt-3 space-y-4">
          <label className="block text-sm font-medium text-[#3c4043]">
            行动
            <select
              className="field-control mt-1"
              disabled={controlsDisabled}
              value={kind}
              onChange={event => {
                clearClassifier();
                setKind(event.target.value as WorldActionKind);
                setTargetId('');
                setDraftScene(fingerprint);
                setSelectedSuggestion(null);
              }}
            >
              <option value="" disabled>请选择行动方式</option>
              {availableActions.map(value => <option key={value} value={value}>{actionLabels[value]}</option>)}
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
                clearClassifier();
                setTargetId(event.target.value);
                setDraftScene(fingerprint);
                setSelectedSuggestion(null);
              }}
              required={targetRequired}
            >
              {!targetRequired ? <option value="">自定目标</option> : null}
              {targetRequired ? <option value="" disabled>请选择目标</option> : null}
              {targetOptions.map(option => <option key={option.id} value={option.id}>{option.name}</option>)}
            </select>
          </label>
          {view.action_suggestions_available ? (
            <div className="text-sm text-[#3c4043]">
              <button type="button" className="min-h-11 underline disabled:opacity-50" disabled={controlsDisabled || suggesting || !intent.trim()} onClick={() => void requestSuggestion()}>
                {suggesting ? '正在分析行动…' : '建议行动类型'}
              </button>
              <p className="mt-1 text-xs text-[#5f6368]">点击后会将当前意图发送至部署方配置的 Laya 服务。</p>
              {classifier?.fingerprint === fingerprint && !controlsDisabled ? (
                <div role="status" className="mt-1">
                  {classifier.kind ? (
                    <>建议：<button type="button" className="min-h-11 underline" onClick={() => {
                      setKind(classifier.kind!);
                      setTargetId('');
                      setDraftScene(fingerprint);
                      setSelectedSuggestion(null);
                    }}>{actionLabels[classifier.kind]}</button>。点击可选用行动类型，再确认目标并提交。</>
                  ) : '暂时没有可靠建议，请自行选择行动。'}
                </div>
              ) : null}
            </div>
          ) : null}
        </div>
      </details>
      <button
        type="submit"
        disabled={controlsDisabled || Boolean(blockingReason)}
        aria-describedby={blockingReason ? guidanceId : undefined}
        className="primary-action"
      >
        {isPending ? '世界正在回应…' : '执行行动'}
      </button>
      {blockingReason ? <p id={guidanceId} className="text-sm text-[#5f6368]">{blockingReason}</p> : null}
    </form>
  );
}
