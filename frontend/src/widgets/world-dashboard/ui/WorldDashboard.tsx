import { displayMessage, translate as t, UiMessageError, useLocale, type UiMessage } from '@/shared/lib/i18n';
import { useEffect, useState, type ReactNode } from 'react';
import { BookOpen, Compass, Dices, GitBranch, History, Users } from 'lucide-react';
import { isWorldTurnOutcomeUnknown, useSubmitWorldTurn, useWorldTurnConfirmation } from '@/entities/narrative';
import { WorldActionForm, actionLabels } from '@/features/world-action';
import { getApiErrorMessage } from '@/shared/api/client';
import { effectiveWorldContext } from '@/shared/lib/worldSourceContext';
import { localWorldCharacterIds } from '@/shared/lib/localWorldCharacters';
import {
  removeWorldTurnPendingRequest,
  worldTurnPendingStorageKey,
} from '@/shared/lib/worldTurnStorage';
import type { ActionCheck, OpenWorldView, WorldAction } from '@/shared/types';

interface WorldDashboardProps {
  novelId: string;
  view: OpenWorldView;
  actionsDisabled?: boolean;
  recoveryOnly?: boolean;
  actionsDisabledReason?: string;
  onRefresh?: () => void;
  onActionLockChange?: (locked: boolean) => void;
  onReviewJournal?: () => void;
  sourceProgressContent?: ReactNode;
}

interface PendingRequest {
  action: WorldAction;
  idempotencyKey: string;
  expectedTurnNumber: number;
  expectedSourceChapter?: number;
}

const maxStoredRequestLength = 4_096;
const pendingRefreshIntervalMs = 10_000;
const actionKinds: WorldAction['kind'][] = [
  'travel',
  'investigate',
  'converse',
  'ally',
  'oppose',
  'advance_thread',
  'resolve_thread',
  'pursue_goal',
];

function isPendingRequest(value: unknown): value is PendingRequest {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const request = value as Record<string, unknown>;
  if ((Object.keys(request).length !== 3 && Object.keys(request).length !== 4)
    || typeof request.idempotencyKey !== 'string'
    || !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i
      .test(request.idempotencyKey)
    || typeof request.expectedTurnNumber !== 'number'
    || !Number.isSafeInteger(request.expectedTurnNumber)
    || request.expectedTurnNumber < 0
    || (request.expectedSourceChapter !== undefined
      && (!Number.isSafeInteger(request.expectedSourceChapter) || Number(request.expectedSourceChapter) < 1))
    || !request.action
    || typeof request.action !== 'object'
    || Array.isArray(request.action)) return false;

  const action = request.action as Record<string, unknown>;
  const targetValid = action.target_id === null || (
    typeof action.target_id === 'string'
    && action.target_id.length > 0
    && [...action.target_id].length <= 200
    && action.target_id.trim() === action.target_id
    && !/[\u0000-\u001f\u007f]/.test(action.target_id)
  );
  return Object.keys(action).length === 3
    && typeof action.kind === 'string'
    && actionKinds.includes(action.kind as WorldAction['kind'])
    && targetValid
    && typeof action.intent === 'string'
    && action.intent.trim() === action.intent
    && action.intent.length > 0
    && [...action.intent].length <= 500
    && !/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/.test(action.intent);
}

function readStoredPendingRequest(userId: string, novelId: string): PendingRequest | null {
  try {
    const stored = window.sessionStorage.getItem(worldTurnPendingStorageKey(userId, novelId));
    if (!stored) return null;
    if (stored.length <= maxStoredRequestLength) {
      const parsed: unknown = JSON.parse(stored);
      if (isPendingRequest(parsed)) return parsed;
    }
  } catch {
    // Invalid or inaccessible storage is treated as absent.
  }
  removeWorldTurnPendingRequest(userId, novelId);
  return null;
}

function storePendingRequest(userId: string, novelId: string, request: PendingRequest) {
  try {
    window.sessionStorage.setItem(
      worldTurnPendingStorageKey(userId, novelId),
      JSON.stringify(request),
    );
  } catch {
    // The in-memory lock still protects the current mount when storage is unavailable.
  }
}

function actionCheckSummary(check: ActionCheck) {
  const decision = check.adjudication?.decision;
  if (decision === 'impossible') return t("Action impossible · No dice check");
  if (decision === 'automatic_success') return t("No check needed · Action succeeded");
  if (decision === 'pending') return t("Decision pending");

  const formula = t("{p0} check: D20 {p1} {p2} {p3} = {p4} / DC {p5}", { p0: check.attribute_label, p1: check.roll, p2: check.modifier >= 0 ? '+' : '−', p3: Math.abs(check.modifier), p4: check.total, p5: check.difficulty_class });
  if (decision === 'easy_check' || decision === 'standard_check' || decision === 'hard_check') {
    const difficulty = decision === 'easy_check' ? t("Low") : decision === 'hard_check' ? t("High") : t("Standard");
    return t("{p0} · {p1} · Assessed difficulty: {p2}", { p0: formula, p1: check.succeeded ? t("Success") : t("Failure"), p2: difficulty });
  }
  return `${formula} · ${check.succeeded ? t("Success") : t("Failure")}${decision === 'template_fallback' ? t(" · Using the template check") : ''}`;
}

function pendingRequestFromView(view: OpenWorldView): PendingRequest | null {
  const entry = view.journal.find(item => item.memory_projection_status === 'pending');
  const request = entry && entry.turn_number >= 1
    ? {
      action: entry.action,
      idempotencyKey: entry.turn_id,
      expectedTurnNumber: entry.turn_number - 1,
      ...(entry.expected_source_chapter == null ? {} : { expectedSourceChapter: entry.expected_source_chapter }),
    }
    : view.recoverable_turn && {
      action: view.recoverable_turn.action,
      idempotencyKey: view.recoverable_turn.turn_id,
      expectedTurnNumber: view.recoverable_turn.expected_turn_number,
      ...(view.recoverable_turn.expected_source_chapter == null ? {} : { expectedSourceChapter: view.recoverable_turn.expected_source_chapter }),
    };
  return isPendingRequest(request) ? request : null;
}

const eventStatus = {
  get scheduled() { return t("Scheduled"); },
  get occurred() { return t("Occurred as in the original"); },
  get witnessed() { return t("Witnessed by the player"); },
  get assisted() { return t("Assisted by the player"); },
  get obstructed() { return t("Obstructed by the player"); },
  get delayed() { return t("Delayed"); },
  get redirected() { return t("Redirected"); },
  get prevented() { return t("Prevented"); },
};

export function WorldDashboard({
  novelId,
  view,
  actionsDisabled = false,
  recoveryOnly = false,
  actionsDisabledReason = t("The latest world state has not been restored. Reload the world before taking an action."),
  onRefresh,
  onActionLockChange,
  onReviewJournal,
  sourceProgressContent,
}: WorldDashboardProps) {
  const locale = useLocale();
  const turn = useSubmitWorldTurn(novelId);
  const storageKey = worldTurnPendingStorageKey(view.player.user_id, novelId);
  const serverPendingRequest = pendingRequestFromView(view);
  const [pendingState, setPendingState] = useState(() => ({
    storageKey,
    request: serverPendingRequest
      ?? readStoredPendingRequest(view.player.user_id, novelId),
  }));
  const restoredPendingRequest = pendingState.storageKey === storageKey
    ? pendingState.request
    : readStoredPendingRequest(view.player.user_id, novelId);
  // The server owns the unresolved authority slot. A stale request from
  // another tab can never overtake its active or committed pending turn.
  const pendingRequest = serverPendingRequest ?? restoredPendingRequest;
  const confirmation = useWorldTurnConfirmation(
    novelId, view.player.user_id, pendingRequest?.idempotencyKey, !turn.isPending,
  );
  useEffect(() => {
    onActionLockChange?.(turn.isPending || Boolean(pendingRequest));
    return () => onActionLockChange?.(false);
  }, [onActionLockChange, pendingRequest, turn.isPending]);
  const pendingEntry = view.journal.find(entry => (
    entry.turn_id === pendingRequest?.idempotencyKey && entry.memory_projection_status === 'pending'
  ));
  const pendingReason = pendingEntry
    ? t("Turn {p0} is saved, but character memories are still syncing. The next turn is paused while the saved status is checked automatically.", { p0: pendingEntry.turn_number })
    : serverPendingRequest
      ? t("The previous action is unfinished. The next turn is paused while its status is checked automatically. Choose “Restore original action” to resume processing.")
      : t("This action's final result is unconfirmed. The next turn is paused while its saved status is checked automatically, preventing duplicate actions.");
  const serverPendingAction = serverPendingRequest?.action;
  const serverPendingKey = serverPendingRequest?.idempotencyKey;
  const serverPendingSource = serverPendingRequest?.expectedSourceChapter;
  const serverPendingRevision = serverPendingRequest?.expectedTurnNumber;
  const [errorState, setErrorState] = useState<{ novelId: string; message?: UiMessage }>(() => ({
    novelId,
  }));
  const error = errorState.novelId === novelId ? displayMessage(errorState.message) : undefined;
  const context = effectiveWorldContext(view.session);
  const waitingEventCount = view.session.canonical_events.filter(event => (
    event.status === 'scheduled' || event.status === 'delayed'
  )).length;
  const location = context.locations.find(item => item.id === view.player.location_id);
  const activeThreads = Object.entries(view.world_state.state.threads ?? {})
    .filter(([, thread]) => thread.status === 'open');
  const choices = view.world_state.state.choices;
  const lastEntry = view.journal[view.journal.length - 1];
  const latestTurn = lastEntry?.turn_number === view.session.turn_number ? lastEntry : undefined;
  const latestNarrative = latestTurn?.transition.rendered_narrative;
  const latestCheck = latestTurn?.resolution;
  const localCharacterIds = localWorldCharacterIds(view);
  const localCharacters = context.characters.filter(character => localCharacterIds.has(character.id));
  const localCharacterEvents = latestTurn?.transition.events.filter(event => (
    event.location_id === view.player.location_id
      && event.actor_character_ids.some(id => localCharacterIds.has(id))
  )) ?? [];

  const rememberPendingRequest = (request: PendingRequest) => {
    storePendingRequest(view.player.user_id, novelId, request);
    setPendingState({ storageKey, request });
  };

  const clearPendingRequest = (turnId: string | undefined = pendingRequest?.idempotencyKey) => {
    const stored = readStoredPendingRequest(view.player.user_id, novelId);
    if (!stored || stored.idempotencyKey === turnId) {
      removeWorldTurnPendingRequest(view.player.user_id, novelId);
    }
    setPendingState(current => current.storageKey === storageKey
      && current.request?.idempotencyKey === turnId
      ? { storageKey, request: null } : current);
  };

  const setError = (message?: UiMessage) => setErrorState({ novelId, message });

  useEffect(() => {
    if (!serverPendingAction || !serverPendingKey || serverPendingRevision === undefined) return;
    const authoritative = {
      action: serverPendingAction,
      idempotencyKey: serverPendingKey,
      expectedTurnNumber: serverPendingRevision,
      ...(serverPendingSource === undefined ? {} : { expectedSourceChapter: serverPendingSource }),
    };
    storePendingRequest(view.player.user_id, novelId, authoritative);
    setPendingState(current => (
      current.storageKey === storageKey
        && JSON.stringify(current.request) === JSON.stringify(authoritative)
        ? current
        : { storageKey, request: authoritative }
    ));
  }, [
    serverPendingAction,
    serverPendingKey,
    serverPendingRevision,
    serverPendingSource,
    novelId,
    storageKey,
    view.player.user_id,
  ]);

  useEffect(() => {
    const data = confirmation.data;
    if (turn.isPending || confirmation.isFetching || confirmation.isError
      || !pendingRequest || !data?.refreshedWorld
      || data.confirmation.turn_id !== pendingRequest.idempotencyKey) return;
    const fresh = data.refreshedWorld;
    if (fresh.player.user_id !== view.player.user_id || fresh.player.novel_id !== novelId) return;
    // Wait until the parent renders the fresh authority snapshot before the
    // form can accept an action against its turn number and source context.
    if (view.world_state.updated_at !== fresh.world_state.updated_at
      || view.session.turn_number !== fresh.session.turn_number) return;
    const authoritative = pendingRequestFromView(fresh);
    if (authoritative) {
      if (authoritative.idempotencyKey !== pendingRequest.idempotencyKey) {
        rememberPendingRequest(authoritative);
      }
      return;
    }
    if (data.confirmation.status === 'completed'
      && fresh.session.turn_number < pendingRequest.expectedTurnNumber + 1) return;
    clearPendingRequest(pendingRequest.idempotencyKey);
    setError(data.confirmation.status === 'failed'
      ? { key: 'The action failed before it changed your world. You can choose a new action.' } : undefined);
  }, [confirmation.data, confirmation.isFetching, confirmation.isError, pendingRequest?.idempotencyKey, storageKey, view, turn.isPending]);

  useEffect(() => {
    if (pendingRequest && view.journal.some(entry => (
      entry.turn_id === pendingRequest.idempotencyKey
      && (entry.memory_projection_status === 'saved'
        || entry.memory_projection_status === 'skipped')
    ))) {
      clearPendingRequest(pendingRequest.idempotencyKey);
      setError(undefined);
    }
  }, [pendingRequest, view.journal]);

  useEffect(() => {
    if (!pendingRequest?.idempotencyKey || !onRefresh) return;
    const interval = window.setInterval(onRefresh, pendingRefreshIntervalMs);
    return () => window.clearInterval(interval);
  }, [onRefresh, pendingRequest?.idempotencyKey]);

  const run = async (request: PendingRequest) => {
    rememberPendingRequest(request);
    setError(undefined);
    try {
      await turn.mutateAsync(request);
      clearPendingRequest(request.idempotencyKey);
    } catch (requestError) {
      const outcomeUnknown = isWorldTurnOutcomeUnknown(requestError);
      if (!outcomeUnknown) clearPendingRequest(request.idempotencyKey);
      setError(requestError instanceof UiMessageError ? requestError.uiMessage
        : getApiErrorMessage(requestError, '')
          || (outcomeUnknown && requestError instanceof Error ? requestError.message : undefined)
          || { key: "World action submission failed" });
      throw requestError;
    }
  };

  const submit = (action: WorldAction) => run({
    action,
    idempotencyKey: crypto.randomUUID(),
    expectedTurnNumber: view.session.turn_number,
    expectedSourceChapter: context.unlocked_through_chapter,
  });

  return (
    <section
      className="mt-6 overflow-hidden rounded-[28px] border border-[#d8c8a9] bg-[#faf7ef] shadow-[0_18px_50px_rgba(53,49,35,0.08)]"
      aria-labelledby="living-world-title"
    >
      <header className="border-b border-[#d8c8a9] bg-[#e8eee5] px-5 py-6 sm:px-8">
        <div className="flex flex-wrap items-center gap-2 text-xs font-semibold uppercase tracking-[0.16em] text-[#466456]">
          <Compass size={14} aria-hidden="true" /> {t("The story unfolding")}
          <span className="rounded-full border border-[#b9c9b7] bg-white/70 px-3 py-1 tracking-normal normal-case text-[#203a35]">
            {t('Turn number: {p0}', { p0: view.session.turn_number })}
          </span>
        </div>
        <h2 id="living-world-title" tabIndex={-1} className="mt-4 scroll-mt-24 text-2xl font-semibold leading-tight text-[#203a35] sm:text-3xl">
          {t("{p0}'s open world", { p0: view.player.name })}
        </h2>
        <div className="mt-4 flex flex-wrap gap-2 text-xs text-[#34483d]">
          <span className="rounded-full border border-[#c8d4c2] bg-white/75 px-3 py-1.5">{location?.name ?? view.player.location_id ?? t("Location unconfirmed")}</span>
          <span className="rounded-full border border-[#c8d4c2] bg-white/75 px-3 py-1.5">{t('World time {p0}', { p0: view.session.world_time })}</span>
          <span className="rounded-full border border-[#c8d4c2] bg-white/75 px-3 py-1.5">{t('Current source · Chapter {p0}', { p0: context.unlocked_through_chapter })}</span>
          <span className="rounded-full border border-[#c8d4c2] bg-white/75 px-3 py-1.5">{t('Events still pending in this scene · {p0}', { p0: waitingEventCount })}</span>
        </div>
      </header>

      <article className="bg-[#fffdf7] px-5 py-7 sm:px-8 sm:py-9">
        {latestCheck ? (
          <div role="status" aria-label={t("This turn's action result")} className="mb-6 rounded-xl border border-[#d8c8a9] bg-[#f5efe1] p-4 text-sm leading-6 text-[#34483d]">
            <p className="font-semibold">{actionCheckSummary(latestCheck)}</p>
            <p className="mt-1">
              {latestCheck.adjudication?.decision === 'impossible'
                ? t("This action is impossible, so no dice check was made. Choose another action or target.")
                : latestCheck.adjudication?.decision === 'pending'
                  ? t("The action decision is still pending. Check the confirmation status in the action section below.")
                  : latestCheck.adjudication?.decision !== 'automatic_success' && !latestCheck.succeeded
                    ? t("Your action failed, but the committed turn still advanced world time.")
                    : t("This turn is complete. You can choose your next action.")}
            </p>
          </div>
        ) : null}
        <p id="latest-world-narrative" lang={latestNarrative ? 'zh-CN' : undefined} role="status" aria-live="polite" tabIndex={-1} className="max-w-3xl whitespace-pre-wrap [font-family:var(--font-reading)] text-lg leading-[1.9] text-[#263c32] [overflow-wrap:anywhere] sm:text-xl">
          {latestNarrative ?? t("The world is ready. Choose a scene suggestion or enter your action. Characters will respond according to their circumstances.")}
        </p>
      </article>

      <div className="border-t border-[#d8c8a9] bg-white/70 px-5 py-6 sm:px-8">
        <h3 id="world-action-form" tabIndex={-1} className="scroll-mt-24 text-lg font-semibold text-[#203a35]">{t("What will you do next?")}</h3>
        {sourceProgressContent ? <div className="my-4 border-l-2 border-[#789381] pl-4">{sourceProgressContent}</div> : null}
        <p className="mb-5 mt-1 text-sm text-[#59645f]">{t("Choose a scene suggestion or write your own. Suggestions only fill your draft; review it before executing. Character suggestions use confirmed nearby characters.")}</p>
        {actionsDisabled ? (
          <div role="alert" className="mb-4 text-sm text-[#b3261e]">
            {actionsDisabledReason}
            {onRefresh ? <button type="button" className="ml-2 underline" onClick={onRefresh}>{t("Retry")}</button> : null}
          </div>
        ) : null}
        {recoveryOnly ? <p role="status" className="mb-3 text-sm text-[#59645f]">{t("The next scene is unconfirmed. Only the original submitted action can be recovered; new actions remain paused.")}</p> : null}
        <p role="status" aria-label={t("World action status")} className="text-sm text-[#59645f]">
          {turn.isPending ? t("Confirming the world action. The next turn will open when it finishes. Wait for this result.") : ''}
        </p>
        {!turn.isPending && (error || pendingRequest) ? (
          <div role="alert" className="mt-4 text-sm text-[#b3261e]">
            {error ? `${error} ` : ''}{pendingRequest
              ? pendingReason
              : t("The request was explicitly rejected. Update your action using the latest world state and try again.")}
            {pendingRequest ? (
              <button className="ml-2 underline" disabled={confirmation.isFetching} onClick={() => void confirmation.refetch()}>
                {t("Check result")}
              </button>
            ) : null}
            {pendingRequest && confirmation.data?.confirmation.status !== 'failed'
              && !(confirmation.data?.confirmation.status === 'completed'
                && confirmation.data.confirmation.memory_projection_status !== 'pending') ? (
              <button className="ml-2 underline" disabled={turn.isPending || actionsDisabled || confirmation.isFetching}
                onClick={() => void run(pendingRequest).catch(() => undefined)}>
                {t("Restore original action")}
              </button>
            ) : null}
          </div>
        ) : null}
        <WorldActionForm
          key={`${view.player.user_id}:${view.player.novel_id}:${view.player.id}`}
          view={view}
          isPending={turn.isPending}
          isLocked={recoveryOnly || actionsDisabled || Boolean(pendingRequest)}
          onSubmit={submit}
        />
        {onReviewJournal ? (
          <button type="button" onClick={onReviewJournal} className="mt-5 inline-flex min-h-11 items-center gap-2 text-sm font-medium text-[#315b45] underline underline-offset-4 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#315b45] focus-visible:ring-offset-2">
            <History size={16} aria-hidden="true" /> {t("Review action journal")}
          </button>
        ) : null}
      </div>

      <div className="space-y-7 border-t border-[#e5dcc9] px-5 py-6 sm:px-8">
      <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(220px,0.75fr)]">
        <div className="rounded-xl border border-[#ded4bf] bg-white p-4">
          <h3 className="flex items-center gap-2 text-sm font-semibold text-[#203a35]">
            <Users size={16} aria-hidden="true" /> {t("Characters here now")}
          </h3>
          {localCharacters.length ? (
            <ul className="mt-4 flex flex-wrap gap-2">
              {localCharacters.map(character => (
                <li key={character.id} className="rounded-full bg-[#e8efe5] px-3 py-1.5 text-sm font-medium text-[#203a35]">
                  {character.name}
                </li>
              ))}
            </ul>
          ) : <p className="mt-3 text-sm leading-6 text-[#59645f]">{t("No characters have been confirmed here by this turn's recorded events.")}</p>}
        </div>
        <div className="rounded-xl border border-[#ded4bf] bg-white p-4">
          <h3 className="text-sm font-semibold text-[#203a35]">{t("What characters are doing")}</h3>
          {localCharacterEvents.length ? (
            <ul className="mt-3 space-y-3 text-sm leading-6 text-[#3d4842]">
              {localCharacterEvents.map((event, index) => (
                <li key={index} className="border-l-2 border-[#81a68d] pl-3">
                  <span className="font-semibold">{event.actor_character_ids
                    .filter(id => localCharacterIds.has(id))
                    .map(id => context.characters.find(character => character.id === id)?.name)
                    .join(locale === 'zh-CN' ? '、' : ', ')}{locale === 'zh-CN' ? '：' : ': '}</span>{event.summary}
                </li>
              ))}
            </ul>
          ) : <p className="mt-3 text-sm leading-6 text-[#59645f]">{t("No actions by nearby characters were recorded this turn.")}</p>}
        </div>
      </div>

      <div className="grid gap-4 md:grid-cols-2">
        {view.session.game_rules && view.player.rules?.mode === 'advanced' ? (
          <div className="rounded-xl border border-[#d2e3fc] bg-[#f8faff] p-4 md:col-span-2">
            <h3 className="flex items-center gap-2 text-sm font-semibold text-[#0b57d0]">
              <Dices size={14} /> {t("Novel attributes")}
            </h3>
            <dl className="mt-3 grid gap-2 sm:grid-cols-3">
              {view.session.game_rules.attributes.map(attribute => (
                <div key={attribute.key} className="rounded-lg bg-white p-3">
                  <dt className="text-xs text-[#5f6368]">{attribute.label}</dt>
                  <dd className="text-lg font-semibold text-[#1f1f1f]">
                    {view.player.rules?.attributes[attribute.key]}
                    {latestTurn?.transition.attribute_changes?.filter(change => change.attribute_key === attribute.key).map((change, index) => (
                      <span key={`${change.event_index}-${index}`} className={`ml-2 text-xs font-medium ${change.delta < 0 ? 'text-[#b3261e]' : 'text-[#0d652d]'}`}>
                        {change.delta > 0 ? '+' : ''}{change.delta} · {change.reason}
                      </span>
                    ))}
                  </dd>
                </div>
              ))}
            </dl>
          </div>
        ) : null}
        <div className="rounded-xl border border-[#d2e3fc] bg-[#f8faff] p-4">
          <h3 className="flex items-center gap-2 text-sm font-semibold text-[#0b57d0]">
            <GitBranch size={14} /> {t("Active threads")}
          </h3>
          {activeThreads.length ? (
            <ul className="mt-3 space-y-2 text-sm text-[#3c4043]">
              {activeThreads.map(([id, thread]) => (
                <li key={id}>{thread.description} <span className="text-xs text-[#5f6368]">· {thread.origin === 'canon' ? t("Original storyline") : thread.origin === 'player' ? t("Created by the player") : t("Source unconfirmed")}</span></li>
              ))}
            </ul>
          ) : <p className="mt-3 text-sm text-[#5f6368]">{t("No active threads")}</p>}
        </div>
        <div className="rounded-xl border border-[#d2e3fc] bg-[#f8faff] p-4">
          <h3 className="flex items-center gap-2 text-sm font-semibold text-[#0b57d0]">
            <Users size={14} /> {t("Character relationships")}
          </h3>
          {Object.keys(view.player.relationships).length ? (
            <ul className="mt-3 space-y-2 text-sm text-[#3c4043]">
              {Object.entries(view.player.relationships).map(([id, relationship]) => (
                <li key={id}>
                  {context.characters.find(character => character.id === id)?.name ?? id}: {relationship.score}
                </li>
              ))}
            </ul>
          ) : <p className="mt-3 text-sm text-[#5f6368]">{t("No relationships yet")}</p>}
        </div>
      </div>

      <div>
        <h3 className="flex items-center gap-2 text-sm font-semibold text-[#1f1f1f]">
          <BookOpen size={14} /> {t("Original event timeline")}
        </h3>
        {view.session.canonical_events.length ? (
          <>
            <p className="mt-2 text-xs text-[#5f6368]">{t("Events are extracted from the original by a model and may be incomplete or misread. Check their source chapters.")}</p>
            <ol className="mt-3 space-y-3">
              {view.session.canonical_events.map(event => (
                <li key={event.id} className="rounded-lg border border-[#e1e3e8] bg-white p-3 text-sm text-[#3c4043]">
                  <span className="mr-2 text-xs font-semibold text-[#0b57d0]">{t("Extracted from the original")}</span>
                  {event.summary}
                  <div className="mt-1 text-xs text-[#5f6368]">
                    {eventStatus[event.status]}{event.advanced_at_world_time != null ? t(" · World time {p0}", { p0: event.advanced_at_world_time }) : ''}{t(' · Source chapters {p0}', { p0: event.source_chapters.join(locale === 'zh-CN' ? '、' : ', ') })}{event.reason ? ` · ${event.reason}` : ''}
                  </div>
                </li>
              ))}
            </ol>
          </>
        ) : <p className="mt-3 text-sm text-[#5f6368]">{t("No original events remain in the current unlocked range.")}</p>}
      </div>

      <div>
        <h3 id="world-action-journal" tabIndex={-1} className="flex scroll-mt-24 items-center gap-2 text-sm font-semibold text-[#1f1f1f]">
          <History size={14} /> {t("Journey timeline")}
        </h3>
        <div role="log" aria-labelledby="world-action-journal" aria-relevant="additions">
        {choices.length || view.journal.length ? (
          <ol className="mt-3 space-y-3">
            {choices.map((choice, index) => (
              <li
                key={choice.node_id ?? `choice-${choice.chapter}-${index}`}
                className="rounded-lg border border-[#d2e3fc] bg-[#f8faff] p-3 text-sm text-[#3c4043]"
              >
                <span className="mr-2 text-xs font-semibold text-[#0b57d0]">{t('Original position · Chapter {p0}', { p0: choice.chapter })}</span>
                <span className="mr-2 text-xs font-semibold text-[#0d652d]">{t("Reader choice")}</span>
                <span className="whitespace-pre-wrap [overflow-wrap:anywhere]">{choice.choice}</span>
                <div className="mt-1 text-xs text-[#5f6368]">
                  <span className="mr-2 font-semibold text-[#0b57d0]">{t("Generated projection")}</span>
                  <span lang="zh-CN" className="whitespace-pre-wrap [overflow-wrap:anywhere]">{choice.consequence}</span>
                </div>
                {choice.timestamp ? (
                  <time dateTime={choice.timestamp} className="mt-1 block text-xs text-[#5f6368]">
                    {new Date(choice.timestamp).toLocaleString(locale)}
                  </time>
                ) : null}
              </li>
            ))}
            {view.journal.map(entry => (
              <li key={entry.turn_id} className="rounded-lg border border-[#d2e3fc] bg-[#f8faff] p-3 text-sm text-[#3c4043]">
                <span className="mr-2 text-xs font-semibold text-[#0b57d0]">{t('Turn {p0}', { p0: entry.turn_number })}</span>
                <span className="mr-2 text-xs font-semibold text-[#0d652d]">{t("Reader action")}</span>
                <span className="whitespace-pre-wrap [overflow-wrap:anywhere]">
                  {t('{p0}: {p1}', { p0: actionLabels[entry.action.kind], p1: entry.action.intent })}
                </span>
                {entry.turn_id === latestTurn?.turn_id ? (
                  <a href="#latest-world-narrative" className="ml-2 text-xs font-medium text-[#0b57d0] underline underline-offset-2">
                    {t("Read this turn's full narrative")}
                  </a>
                ) : null}
                {entry.resolution ? (
                  <div className={`mt-2 text-xs font-semibold ${entry.resolution.adjudication?.decision === 'impossible'
                    ? 'text-[#b3261e]'
                    : entry.resolution.adjudication?.decision === 'pending'
                      ? 'text-[#8a4b08]'
                      : entry.resolution.adjudication?.decision === 'automatic_success' || entry.resolution.succeeded
                        ? 'text-[#0d652d]'
                        : 'text-[#b3261e]'}`}>
                    {actionCheckSummary(entry.resolution)}
                  </div>
                ) : null}
                {entry.transition.attribute_changes?.length ? (
                  <ul className="mt-2 space-y-1 text-xs">
                    {entry.transition.attribute_changes.map((change, index) => {
                      const attribute = view.session.game_rules?.attributes.find(item => item.key === change.attribute_key);
                      const event = entry.transition.events[change.event_index];
                      return (
                        <li key={`${change.event_index}-${change.attribute_key}-${index}`} className={change.delta < 0 ? 'text-[#b3261e]' : 'text-[#0d652d]'}>
                          {event ? `${event.summary}：` : ''}{attribute?.label ?? change.attribute_key} {change.delta > 0 ? '+' : ''}{change.delta} · {change.reason}
                        </li>
                      );
                    })}
                  </ul>
                ) : null}
                {entry.turn_id !== latestTurn?.turn_id ? <div className="mt-1 text-xs text-[#5f6368]">
                  <span className="mr-2 font-semibold text-[#0b57d0]">{t("Generated projection")}</span>
                  <span lang="zh-CN" className="whitespace-pre-wrap [overflow-wrap:anywhere]">
                    {entry.transition.rendered_narrative}
                  </span>
                </div> : null}
                <time dateTime={entry.completed_at} className="mt-1 block text-xs text-[#5f6368]">
                  {new Date(entry.completed_at).toLocaleString(locale)}
                </time>
              </li>
            ))}
          </ol>
        ) : <p className="mt-3 text-sm text-[#5f6368]">{t("Your first choice or action will be recorded here.")}</p>}
        </div>
      </div>
      </div>

    </section>
  );
}
