import { useEffect, useState } from 'react';
import { BookOpen, Compass, Dices, GitBranch, History, Users } from 'lucide-react';
import { isWorldTurnOutcomeUnknown, useSubmitWorldTurn } from '@/entities/narrative';
import { WorldActionForm, actionLabels } from '@/features/world-action';
import { getApiErrorMessage } from '@/shared/api/client';
import {
  removeWorldTurnPendingRequest,
  worldTurnPendingStorageKey,
} from '@/shared/lib/worldTurnStorage';
import type { OpenWorldView, WorldAction } from '@/shared/types';

interface WorldDashboardProps {
  novelId: string;
  view: OpenWorldView;
  actionsDisabled?: boolean;
}

interface PendingRequest {
  action: WorldAction;
  idempotencyKey: string;
  expectedTurnNumber: number;
}

const maxStoredRequestLength = 4_096;
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
  if (Object.keys(request).length !== 3
    || typeof request.idempotencyKey !== 'string'
    || !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i
      .test(request.idempotencyKey)
    || typeof request.expectedTurnNumber !== 'number'
    || !Number.isSafeInteger(request.expectedTurnNumber)
    || request.expectedTurnNumber < 0
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

function pendingRequestFromView(view: OpenWorldView): PendingRequest | null {
  const entry = view.journal.find(item => item.memory_projection_status === 'pending');
  if (!entry || entry.turn_number < 1) return null;
  const request: PendingRequest = {
    action: entry.action,
    idempotencyKey: entry.turn_id,
    expectedTurnNumber: entry.turn_number - 1,
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

export function WorldDashboard({ novelId, view, actionsDisabled = false }: WorldDashboardProps) {
  const turn = useSubmitWorldTurn(novelId);
  const storageKey = worldTurnPendingStorageKey(view.player.user_id, novelId);
  const journalPendingRequest = pendingRequestFromView(view);
  const [pendingState, setPendingState] = useState(() => ({
    storageKey,
    request: journalPendingRequest
      ?? readStoredPendingRequest(view.player.user_id, novelId),
  }));
  const restoredPendingRequest = pendingState.storageKey === storageKey
    ? pendingState.request
    : readStoredPendingRequest(view.player.user_id, novelId);
  // The server journal owns the unresolved authority slot. A stale request
  // from another tab can never overtake a different committed pending turn.
  const pendingRequest = journalPendingRequest ?? restoredPendingRequest;
  const journalPendingAction = journalPendingRequest?.action;
  const journalPendingKey = journalPendingRequest?.idempotencyKey;
  const journalPendingRevision = journalPendingRequest?.expectedTurnNumber;
  const [errorState, setErrorState] = useState<{ novelId: string; message?: string }>(() => ({
    novelId,
  }));
  const error = errorState.novelId === novelId ? errorState.message : undefined;
  const { entry_context: context } = view.session;
  const location = context.locations.find(item => item.id === view.player.location_id);
  const activeThreads = Object.entries(view.world_state.state.threads ?? {})
    .filter(([, thread]) => thread.status === 'open');
  const choices = view.world_state.state.choices;

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
    if (!journalPendingAction || !journalPendingKey || journalPendingRevision === undefined) return;
    const authoritative = {
      action: journalPendingAction,
      idempotencyKey: journalPendingKey,
      expectedTurnNumber: journalPendingRevision,
    };
    storePendingRequest(view.player.user_id, novelId, authoritative);
    setPendingState(current => (
      current.storageKey === storageKey
        && JSON.stringify(current.request) === JSON.stringify(authoritative)
        ? current
        : { storageKey, request: authoritative }
    ));
  }, [
    journalPendingAction,
    journalPendingKey,
    journalPendingRevision,
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

  const run = async (request: PendingRequest) => {
    rememberPendingRequest(request);
    setError(undefined);
    try {
      await turn.mutateAsync(request);
      clearPendingRequest();
    } catch (requestError) {
      const outcomeUnknown = isWorldTurnOutcomeUnknown(requestError);
      if (!outcomeUnknown) clearPendingRequest();
      setError(getApiErrorMessage(requestError, '世界行动提交失败'));
      throw requestError;
    }
  };

  const submit = (action: WorldAction) => run({
    action,
    idempotencyKey: crypto.randomUUID(),
    expectedTurnNumber: view.session.turn_number,
  });

  return (
    <section
      className="surface-card mt-12 space-y-8 p-5 md:p-6"
      aria-labelledby="living-world-title"
    >
      <div>
        <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-widest text-[#0b57d0]">
          <Compass size={14} /> 单一时间线
        </div>
        <h2 id="living-world-title" tabIndex={-1} className="mt-2 scroll-mt-24 text-xl font-semibold text-[#1f1f1f]">
          {view.player.name} 的开放世界
        </h2>
        <p className="mt-2 text-sm text-[#5f6368]">
          世界时间 {view.session.world_time} · 已完成 {view.session.turn_number} 回合 · 当前地点 {location?.name ?? view.player.location_id}
        </p>
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
                  <dd className="text-lg font-semibold text-[#1f1f1f]">{view.player.rules?.attributes[attribute.key]}</dd>
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
                <li key={id}>{thread.description} <span className="text-xs text-[#5f6368]">· {thread.origin === 'player' ? '玩家创造' : '原著主线'}</span></li>
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
          <ol className="mt-3 space-y-3">
            {view.session.canonical_events.map(event => (
              <li key={event.id} className="rounded-lg border border-[#e1e3e8] bg-white p-3 text-sm text-[#3c4043]">
                <span className="mr-2 text-xs font-semibold text-[#0b57d0]">原著主线</span>
                {event.summary}
                <div className="mt-1 text-xs text-[#5f6368]">
                  {eventStatus[event.status]} · 来源章节 {event.source_chapters.join('、')}{event.reason ? ` · ${event.reason}` : ''}
                </div>
              </li>
            ))}
          </ol>
        ) : <p className="mt-3 text-sm text-[#5f6368]">当前解锁范围内没有待运行的原著事件。</p>}
      </div>

      <div>
        <h3 id="world-action-journal" tabIndex={-1} className="flex scroll-mt-24 items-center gap-2 text-sm font-semibold text-[#1f1f1f]">
          <History size={14} /> 旅程时间线
        </h3>
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
                {entry.resolution ? (
                  <div className={`mt-2 text-xs font-semibold ${entry.resolution.succeeded ? 'text-[#0d652d]' : 'text-[#b3261e]'}`}>
                    {entry.resolution.attribute_label}检定：D20 {entry.resolution.roll} {entry.resolution.modifier >= 0 ? '+' : '−'} {Math.abs(entry.resolution.modifier)} = {entry.resolution.total} / 难度 {entry.resolution.difficulty_class} · {entry.resolution.succeeded ? '成功' : '失败'}
                  </div>
                ) : null}
                <div className="mt-1 text-xs text-[#5f6368]">
                  <span className="mr-2 font-semibold text-[#0b57d0]">生成投影</span>
                  <span className="whitespace-pre-wrap [overflow-wrap:anywhere]">
                    {entry.transition.rendered_narrative}
                  </span>
                </div>
                <time dateTime={entry.completed_at} className="mt-1 block text-xs text-[#5f6368]">
                  {entry.completed_at}
                </time>
              </li>
            ))}
          </ol>
        ) : <p className="mt-3 text-sm text-[#5f6368]">你的第一个选择或行动将记录在这里。</p>}
      </div>

      <div className="border-t border-[#e1e3e8] pt-6">
        <h3 id="world-action-form" tabIndex={-1} className="mb-4 scroll-mt-24 text-sm font-semibold text-[#1f1f1f]">采取下一步行动</h3>
        <WorldActionForm
          view={view}
          isPending={turn.isPending}
          isLocked={actionsDisabled || Boolean(pendingRequest)}
          onSubmit={submit}
        />
        {error || pendingRequest ? (
          <div role="alert" className="mt-4 text-sm text-[#b3261e]">
            {error ? `${error} ` : ''}{pendingRequest
              ? '尚未确认这次行动的最终结果；请使用原请求继续确认，避免重复行动。'
              : '请求已被明确拒绝；请根据最新世界状态修改行动后重试。'}
            {pendingRequest ? (
              <button className="ml-2 underline" disabled={turn.isPending || actionsDisabled} onClick={() => void run(pendingRequest).catch(() => undefined)}>
                继续确认结果
              </button>
            ) : null}
          </div>
        ) : null}
      </div>
    </section>
  );
}
