import { translate as t, useLocale } from '@/shared/lib/i18n';
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
  get travel() { return t("Travel to a location"); },
  get investigate() { return t("Investigate a clue"); },
  get converse() { return t("Converse with a character"); },
  get ally() { return t("Seek an alliance"); },
  get oppose() { return t("Oppose openly"); },
  get advance_thread() { return t("Advance a thread"); },
  get resolve_thread() { return t("Resolve a thread (legacy)"); },
  get pursue_goal() { return t("Pursue your own goal"); },
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
    // Keep internal target names stable when only the interface language changes.
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
    { label: t("Look around"), intent: t("Look around and see what deserves your attention.") },
    { label: t("Review clues"), intent: t("Review what you know and plan your next move.") },
    { label: t("Plan your next move"), intent: t("Consider the situation and decide what to do next.") },
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

  add('converse', localCharacter, t("Talk to {p0}", { p0: localCharacter?.name ?? '' }), t("Try talking to {p0} about the situation.", { p0: localCharacter?.name ?? '' }));
  add('investigate', investigateTarget, t("Check on {p0}", { p0: investigateSubject ?? '' }), t("Check on {p0} and plan your next move.", { p0: investigateSubject ?? t("Current surroundings") }));
  add('travel', nextLocation, t("Travel to {p0}", { p0: nextLocation?.name ?? '' }), t("Travel to {p0} and see what is happening there.", { p0: nextLocation?.name ?? t("Elsewhere") }));
  return suggestions;
}

export function WorldActionForm({ view, isPending, isLocked = false, onSubmit }: WorldActionFormProps) {
  const locale = useLocale();
  const fingerprint = useMemo(() => sceneFingerprint(view), [view]);
  const suggestions = useMemo(() => sceneSuggestions(view), [view, locale]);
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
    : sceneChanged ? t("The scene changed. Choose a new suggestion or confirm that this draft still fits the current scene.")
      : !kind ? t("Choose a scene suggestion, write freely or adjust the action type.")
        : targetRequired && targetOptions.length === 0
          ? t("No target is available for this action. Write freely to describe your next move.")
          : targetRequired && !selectedTarget ? t("Choose the target for this action first.")
            : !intent.trim() ? t("Describe what you want to do in “Your intent”.") : undefined;
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
        {t('You act as your original character “{p0}”; source characters respond according to their own goals.', { p0: view.player.name })}
      </p>
      <fieldset className="space-y-2">
        <legend className="mb-2 text-sm font-medium text-[#3c4043]">{t("Scene suggestions")}</legend>
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
        {t("Write freely")}
      </button>
      <label className="block text-sm font-medium text-[#3c4043]">
        {t("Your intent")}
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
          {t("The scene changed. Choose a new suggestion or confirm this draft.")}
          <button type="button" className="ml-2 min-h-11 rounded px-2 font-medium underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#315b45]" onClick={() => setDraftScene(fingerprint)}>
            {t("Confirm for the current scene")}
          </button>
        </div>
      ) : null}
      <details className="rounded-lg border border-[#ded6c4] bg-white/70 p-3">
        <summary className="cursor-pointer rounded text-sm font-medium text-[#315b45] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#315b45]">{t("Adjust action type and target")}</summary>
        <div className="mt-3 space-y-4">
          <label className="block text-sm font-medium text-[#3c4043]">
            {t("Action")}
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
              <option value="" disabled>{t("Choose an action type")}</option>
              {availableActions.map(value => <option key={value} value={value}>{actionLabels[value]}</option>)}
            </select>
          </label>
          {actionRule && actionAttribute && actionScore !== undefined && actionModifier !== undefined ? (
            <div className="rounded-lg border border-[#d2e3fc] bg-[#f8faff] p-3 text-sm text-[#3c4043]">
              <span className="font-semibold text-[#0b57d0]">{t("Check preview")}</span>
              <span className="ml-2">D20 + {actionAttribute.label} {actionModifier >= 0 ? `+${actionModifier}` : actionModifier}{t(", template base difficulty")} {actionRule.difficulty_class}</span>
              <p className="mt-1 text-xs text-[#5f6368]">
                {actionRule.description}{t(". The server assesses intent after submission; a check may be unnecessary or its difficulty may change. If configuration is unavailable, it uses the template check. The actual result is saved and replayable.")}
              </p>
            </div>
          ) : null}
          <label className="block text-sm font-medium text-[#3c4043]">
            {t("Target")}{targetRequired ? '' : t("(optional)")}
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
              {!targetRequired ? <option value="">{t("Custom target")}</option> : null}
              {targetRequired ? <option value="" disabled>{t("Choose a target")}</option> : null}
              {targetOptions.map(option => <option key={option.id} value={option.id}>{option.name.startsWith('主线事件：') ? t('Main event: {p0}', { p0: option.name.slice(5) }) : option.name.startsWith('事件线：') ? t('Thread: {p0}', { p0: option.name.slice(4) }) : option.name}</option>)}
            </select>
          </label>
          {view.action_suggestions_available ? (
            <div className="text-sm text-[#3c4043]">
              <button type="button" className="min-h-11 underline disabled:opacity-50" disabled={controlsDisabled || suggesting || !intent.trim()} onClick={() => void requestSuggestion()}>
                {suggesting ? t("Analyzing action…") : t("Suggest an action type")}
              </button>
              <p className="mt-1 text-xs text-[#5f6368]">{t("Sends the current intent to the deployment's configured Laya service when clicked.")}</p>
              {classifier?.fingerprint === fingerprint && !controlsDisabled ? (
                <div role="status" className="mt-1">
                  {classifier.kind ? (
                    <>{t("Suggestion:")}<button type="button" className="min-h-11 underline" onClick={() => {
                      setKind(classifier.kind!);
                      setTargetId('');
                      setDraftScene(fingerprint);
                      setSelectedSuggestion(null);
                    }}>{actionLabels[classifier.kind]}</button>{t(". Select the action type, then confirm a target and submit.")}</>
                  ) : t("No reliable suggestion is available. Choose the action yourself.")}
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
        {isPending ? t("The world is responding…") : t("Execute action")}
      </button>
      {blockingReason ? <p id={guidanceId} className="text-sm text-[#5f6368]">{blockingReason}</p> : null}
    </form>
  );
}
