import { useEffect, useState } from 'react';
import { BookOpen, Compass, Dices, GitBranch, History, Users } from 'lucide-react';
import { isWorldTurnOutcomeUnknown, useSubmitWorldTurn } from '@/entities/narrative';
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
  actionsDisabledReason?: string;
  onRefresh?: () => void;
  onActionLockChange?: (locked: boolean) => void;
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
  if (decision === 'impossible') return '行动不可行 · 未进行骰子检定';
  if (decision === 'automatic_success') return '无需检定 · 行动成功';
  if (decision === 'pending') return '判断未完成';

  const formula = `${check.attribute_label}检定：D20 ${check.roll} ${check.modifier >= 0 ? '+' : '−'} ${Math.abs(check.modifier)} = ${check.total} / 难度 ${check.difficulty_class}`;
  if (decision === 'easy_check' || decision === 'standard_check' || decision === 'hard_check') {
    const difficulty = decision === 'easy_check' ? '低' : decision === 'hard_check' ? '高' : '标准';
    return `${formula} · ${check.succeeded ? '成功' : '失败'} · 语义难度：${difficulty}`;
  }
  return `${formula} · ${check.succeeded ? '成功' : '失败'}${decision === 'template_fallback' ? ' · 沿用模板检定' : ''}`;
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
  scheduled: '等待发生',
  occurred: '如原著发生',
  witnessed: '玩家见证',
  assisted: '玩家协助',
  obstructed: '玩家阻碍',
  delayed: '被延迟',
  redirected: '被改道',
  prevented: '被阻止',
};

export function WorldDashboard({
  novelId,
  view,
  actionsDisabled = false,
  actionsDisabledReason = '最新世界状态尚未恢复，暂时不能执行行动。请重新加载世界后再试。',
  onRefresh,
  onActionLockChange,
}: WorldDashboardProps) {
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
  useEffect(() => {
    onActionLockChange?.(turn.isPending || Boolean(pendingRequest));
    return () => onActionLockChange?.(false);
  }, [onActionLockChange, pendingRequest, turn.isPending]);
  const pendingEntry = view.journal.find(entry => (
    entry.turn_id === pendingRequest?.idempotencyKey && entry.memory_projection_status === 'pending'
  ));
  const pendingReason = pendingEntry
    ? `第 ${pendingEntry.turn_number} 回合的经过已保存，但角色记忆尚未同步完成，因此暂时不能发起下一回合。请点击“继续确认结果”。`
    : serverPendingRequest
      ? '上一行动尚未完成，因此暂时不能发起下一回合。请点击“继续确认结果”恢复原行动。'
      : '尚未确认这次行动的最终结果，因此暂时不能发起下一回合。请点击“继续确认结果”，避免重复行动。';
  const serverPendingAction = serverPendingRequest?.action;
  const serverPendingKey = serverPendingRequest?.idempotencyKey;
  const serverPendingSource = serverPendingRequest?.expectedSourceChapter;
  const serverPendingRevision = serverPendingRequest?.expectedTurnNumber;
  const [errorState, setErrorState] = useState<{ novelId: string; message?: string }>(() => ({
    novelId,
  }));
  const error = errorState.novelId === novelId ? errorState.message : undefined;
  const context = effectiveWorldContext(view.session);
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

  const clearPendingRequest = () => {
    removeWorldTurnPendingRequest(view.player.user_id, novelId);
    setPendingState({ storageKey, request: null });
  };

  const setError = (message?: string) => setErrorState({ novelId, message });

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
    if (pendingRequest && view.journal.some(entry => (
      entry.turn_id === pendingRequest.idempotencyKey
      && (entry.memory_projection_status === 'saved'
        || entry.memory_projection_status === 'skipped')
    ))) {
      clearPendingRequest();
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
      clearPendingRequest();
    } catch (requestError) {
      const outcomeUnknown = isWorldTurnOutcomeUnknown(requestError);
      if (!outcomeUnknown) clearPendingRequest();
      setError(getApiErrorMessage(requestError, outcomeUnknown && requestError instanceof Error
        ? requestError.message : '世界行动提交失败'));
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
      className="mt-6 space-y-7 rounded-[28px] border border-[#d8c8a9] bg-[#faf7ef] p-4 shadow-[0_18px_50px_rgba(53,49,35,0.08)] sm:p-6"
      aria-labelledby="living-world-title"
    >
      <div className="rounded-[22px] bg-[#203a35] px-5 py-7 text-[#f6f1e6] sm:px-8 sm:py-9">
        <div className="flex flex-wrap items-center gap-2 text-xs font-semibold tracking-[0.18em] text-[#d4e4c6]">
          <Compass size={14} aria-hidden="true" /> 正在发生的故事
          <span className="ml-auto rounded-full border border-white/25 px-3 py-1 tracking-normal text-[#f6f1e6]">
            第 {view.session.turn_number} 回合
          </span>
        </div>
        <h2 id="living-world-title" tabIndex={-1} className="mt-5 scroll-mt-24 text-2xl font-semibold leading-tight sm:text-3xl">
          {view.player.name} 的开放世界
        </h2>
        <p className="mt-3 text-sm text-[#d5e1d7]">
          {location?.name ?? view.player.location_id ?? '地点未确认'} · 世界时间 {view.session.world_time} · 每次已提交回合推进 1 步
        </p>
        <p className="mt-2 text-sm text-[#d5e1d7]">
          世界入场坐标 · 原著第 {view.session.entry_context.checkpoint_chapter} 章。当前世界已接入至第 {context.unlocked_through_chapter} 章。
        </p>
        <div className="mt-7 border-t border-white/20 pt-6">
          {latestCheck ? (
            <div role="status" aria-label="本回合行动结果" className="mb-5 rounded-xl border border-white/25 p-4 text-sm leading-6">
              <p className="font-semibold">{actionCheckSummary(latestCheck)}</p>
              <p className="mt-1 text-[#d5e1d7]">
                {latestCheck.adjudication?.decision === 'impossible'
                  ? '该行动不可行，未进行骰子检定；请改选行动方式或目标。'
                  : latestCheck.adjudication?.decision === 'pending'
                    ? '行动判断尚未完成，请查看下方行动区的确认状态。'
                    : latestCheck.adjudication?.decision !== 'automatic_success' && !latestCheck.succeeded
                      ? '本次检定失败，未产生玩家行动效果；回合已结束，你仍可选择下一步行动。'
                      : '本回合已完成，你可以选择下一步行动。'}
              </p>
            </div>
          ) : null}
          <p id="latest-world-narrative" role="status" aria-live="polite" tabIndex={-1} className="mt-3 max-w-3xl whitespace-pre-wrap text-base leading-8 text-[#f6f1e6] [overflow-wrap:anywhere] sm:text-lg">
            {latestNarrative ?? '世界已经就绪。选定行动与目标，故事中的人物会按各自的处境作出回应。'}
          </p>
          <div className="mt-5 text-sm leading-6 text-[#d5e1d7]">
            <p>故事会在你执行下一次行动后推进；选择行动、确认目标并填写意图，再点击“执行行动”。</p>
            <a href="#world-action-form" className="mt-2 inline-block font-semibold text-[#f6f1e6] underline underline-offset-4">去选择行动</a>
          </div>
        </div>
      </div>

      <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(220px,0.75fr)]">
        <div className="rounded-2xl border border-[#ded4bf] bg-white p-5">
          <h3 className="flex items-center gap-2 text-sm font-semibold text-[#203a35]">
            <Users size={16} aria-hidden="true" /> 此刻同场的角色
          </h3>
          {localCharacters.length ? (
            <ul className="mt-4 flex flex-wrap gap-2">
              {localCharacters.map(character => (
                <li key={character.id} className="rounded-full bg-[#e8efe5] px-3 py-1.5 text-sm font-medium text-[#203a35]">
                  {character.name}
                </li>
              ))}
            </ul>
          ) : <p className="mt-3 text-sm leading-6 text-[#59645f]">还没有能由本回合现场事件确认的角色。</p>}
        </div>
        <div className="rounded-2xl border border-[#ded4bf] bg-white p-5">
          <h3 className="text-sm font-semibold text-[#203a35]">角色正在做什么</h3>
          {localCharacterEvents.length ? (
            <ul className="mt-3 space-y-3 text-sm leading-6 text-[#3d4842]">
              {localCharacterEvents.map((event, index) => (
                <li key={index} className="border-l-2 border-[#81a68d] pl-3">
                  <span className="font-semibold">{event.actor_character_ids
                    .filter(id => localCharacterIds.has(id))
                    .map(id => context.characters.find(character => character.id === id)?.name)
                    .join('、')}：</span>{event.summary}
                </li>
              ))}
            </ul>
          ) : <p className="mt-3 text-sm leading-6 text-[#59645f]">本回合没有已记录的同场角色动作。</p>}
        </div>
      </div>

      <div className="rounded-2xl border border-[#d8c8a9] bg-white p-5 sm:p-6">
        <h3 id="world-action-form" tabIndex={-1} className="scroll-mt-24 text-lg font-semibold text-[#203a35]">你接下来做什么？</h3>
        <p className="mb-5 mt-1 text-sm text-[#59645f]">先选择行动方式，再选择目标；只有已确认同场的角色会成为人物目标。</p>
        {actionsDisabled ? (
          <div role="alert" className="mb-4 text-sm text-[#b3261e]">
            {actionsDisabledReason}
            {onRefresh ? <button type="button" className="ml-2 underline" onClick={onRefresh}>重试</button> : null}
          </div>
        ) : null}
        <p role="status" aria-label="世界行动状态" className="text-sm text-[#59645f]">
          {turn.isPending ? '正在确认世界行动，完成后才会开放下一回合；请等待本次结果。' : ''}
        </p>
        {!turn.isPending && (error || pendingRequest) ? (
          <div role="alert" className="mt-4 text-sm text-[#b3261e]">
            {error ? `${error} ` : ''}{pendingRequest
              ? pendingReason
              : '请求已被明确拒绝；请根据最新世界状态修改行动后重试。'}
            {pendingRequest ? (
              <button className="ml-2 underline" disabled={turn.isPending || actionsDisabled} onClick={() => void run(pendingRequest).catch(() => undefined)}>
                继续确认结果
              </button>
            ) : null}
          </div>
        ) : null}
        <WorldActionForm
          view={view}
          isPending={turn.isPending}
          isLocked={actionsDisabled || Boolean(pendingRequest)}
          onSubmit={submit}
        />
      </div>

      <div className="grid gap-4 md:grid-cols-2">
        {view.session.game_rules && view.player.rules?.mode === 'advanced' ? (
          <div className="rounded-xl border border-[#d2e3fc] bg-[#f8faff] p-4 md:col-span-2">
            <h3 className="flex items-center gap-2 text-sm font-semibold text-[#0b57d0]">
              <Dices size={14} /> 小说属性
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
            <GitBranch size={14} /> 活跃事件线
          </h3>
          {activeThreads.length ? (
            <ul className="mt-3 space-y-2 text-sm text-[#3c4043]">
              {activeThreads.map(([id, thread]) => (
                <li key={id}>{thread.description} <span className="text-xs text-[#5f6368]">· {thread.origin === 'canon' ? '原著主线' : thread.origin === 'player' ? '玩家创造' : '来源未确认'}</span></li>
              ))}
            </ul>
          ) : <p className="mt-3 text-sm text-[#5f6368]">暂无活跃事件线</p>}
        </div>
        <div className="rounded-xl border border-[#d2e3fc] bg-[#f8faff] p-4">
          <h3 className="flex items-center gap-2 text-sm font-semibold text-[#0b57d0]">
            <Users size={14} /> 角色关系
          </h3>
          {Object.keys(view.player.relationships).length ? (
            <ul className="mt-3 space-y-2 text-sm text-[#3c4043]">
              {Object.entries(view.player.relationships).map(([id, relationship]) => (
                <li key={id}>
                  {context.characters.find(character => character.id === id)?.name ?? id}: {relationship.score}
                </li>
              ))}
            </ul>
          ) : <p className="mt-3 text-sm text-[#5f6368]">尚未建立关系</p>}
        </div>
      </div>

      <div>
        <h3 className="flex items-center gap-2 text-sm font-semibold text-[#1f1f1f]">
          <BookOpen size={14} /> 原著事件时间线
        </h3>
        {view.session.canonical_events.length ? (
          <>
            <p className="mt-2 text-xs text-[#5f6368]">事件由模型从原著中抽取，可能存在遗漏或误读，请结合来源章节核对。</p>
            <ol className="mt-3 space-y-3">
              {view.session.canonical_events.map(event => (
                <li key={event.id} className="rounded-lg border border-[#e1e3e8] bg-white p-3 text-sm text-[#3c4043]">
                  <span className="mr-2 text-xs font-semibold text-[#0b57d0]">原著抽取</span>
                  {event.summary}
                  <div className="mt-1 text-xs text-[#5f6368]">
                    {eventStatus[event.status]}{event.advanced_at_world_time != null ? ` · 世界时间 ${event.advanced_at_world_time}` : ''} · 来源章节 {event.source_chapters.join('、')}{event.reason ? ` · ${event.reason}` : ''}
                  </div>
                </li>
              ))}
            </ol>
          </>
        ) : <p className="mt-3 text-sm text-[#5f6368]">当前解锁范围内没有待运行的原著事件。</p>}
      </div>

      <div>
        <h3 id="world-action-journal" tabIndex={-1} className="flex scroll-mt-24 items-center gap-2 text-sm font-semibold text-[#1f1f1f]">
          <History size={14} /> 旅程时间线
        </h3>
        <div role="log" aria-labelledby="world-action-journal" aria-relevant="additions">
        {choices.length || view.journal.length ? (
          <ol className="mt-3 space-y-3">
            {choices.map((choice, index) => (
              <li
                key={choice.node_id ?? `choice-${choice.chapter}-${index}`}
                className="rounded-lg border border-[#d2e3fc] bg-[#f8faff] p-3 text-sm text-[#3c4043]"
              >
                <span className="mr-2 text-xs font-semibold text-[#0b57d0]">原著坐标 · 第 {choice.chapter} 章</span>
                <span className="mr-2 text-xs font-semibold text-[#0d652d]">读者选择</span>
                <span className="whitespace-pre-wrap [overflow-wrap:anywhere]">{choice.choice}</span>
                <div className="mt-1 text-xs text-[#5f6368]">
                  <span className="mr-2 font-semibold text-[#0b57d0]">生成投影</span>
                  <span className="whitespace-pre-wrap [overflow-wrap:anywhere]">{choice.consequence}</span>
                </div>
                {choice.timestamp ? (
                  <time dateTime={choice.timestamp} className="mt-1 block text-xs text-[#5f6368]">
                    {choice.timestamp}
                  </time>
                ) : null}
              </li>
            ))}
            {view.journal.map(entry => (
              <li key={entry.turn_id} className="rounded-lg border border-[#d2e3fc] bg-[#f8faff] p-3 text-sm text-[#3c4043]">
                <span className="mr-2 text-xs font-semibold text-[#0b57d0]">回合 {entry.turn_number}</span>
                <span className="mr-2 text-xs font-semibold text-[#0d652d]">读者行动</span>
                <span className="whitespace-pre-wrap [overflow-wrap:anywhere]">
                  {actionLabels[entry.action.kind]}：{entry.action.intent}
                </span>
                {entry.turn_id === latestTurn?.turn_id ? (
                  <a href="#latest-world-narrative" className="ml-2 text-xs font-medium text-[#0b57d0] underline underline-offset-2">
                    查看本回合完整叙事
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
                  <span className="mr-2 font-semibold text-[#0b57d0]">生成投影</span>
                  <span className="whitespace-pre-wrap [overflow-wrap:anywhere]">
                    {entry.transition.rendered_narrative}
                  </span>
                </div> : null}
                <time dateTime={entry.completed_at} className="mt-1 block text-xs text-[#5f6368]">
                  {entry.completed_at}
                </time>
              </li>
            ))}
          </ol>
        ) : <p className="mt-3 text-sm text-[#5f6368]">你的第一个选择或行动将记录在这里。</p>}
        </div>
      </div>

    </section>
  );
}
