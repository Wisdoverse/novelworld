import { useState } from 'react';
import { BookOpen, Compass, GitBranch, History, Users } from 'lucide-react';
import { useSubmitWorldTurn } from '@/entities/narrative/api';
import { WorldActionForm, actionLabels } from '@/features/world-action/ui/WorldActionForm';
import { getApiErrorMessage } from '@/shared/api/client';
import type { OpenWorldView, WorldAction } from '@/shared/types';

interface WorldDashboardProps {
  novelId: string;
  view: OpenWorldView;
}

interface PendingRequest {
  action: WorldAction;
  idempotencyKey: string;
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

export function WorldDashboard({ novelId, view }: WorldDashboardProps) {
  const turn = useSubmitWorldTurn(novelId);
  const [pendingRequest, setPendingRequest] = useState<PendingRequest | null>(null);
  const [error, setError] = useState<string>();
  const { entry_context: context } = view.session;
  const location = context.locations.find(item => item.id === view.player.location_id);
  const activeThreads = Object.entries(view.world_state.state.threads ?? {})
    .filter(([, thread]) => thread.status === 'open');

  const run = async (request: PendingRequest) => {
    setPendingRequest(request);
    setError(undefined);
    try {
      await turn.mutateAsync(request);
      setPendingRequest(null);
    } catch (requestError) {
      setError(getApiErrorMessage(requestError, '世界行动提交失败'));
      throw requestError;
    }
  };

  const submit = (action: WorldAction) => run({
    action,
    idempotencyKey: crypto.randomUUID(),
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
        <h2 id="living-world-title" className="mt-2 text-xl font-semibold text-[#1f1f1f]">
          {view.player.name} 的开放世界
        </h2>
        <p className="mt-2 text-sm text-[#5f6368]">
          世界时间 {view.session.world_time} · 已完成 {view.session.turn_number} 回合 · 当前地点 {location?.name ?? view.player.location_id}
        </p>
      </div>

      <div className="grid gap-4 md:grid-cols-2">
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
        <h3 className="flex items-center gap-2 text-sm font-semibold text-[#1f1f1f]">
          <History size={14} /> 玩家行动日志
        </h3>
        {view.journal.length ? (
          <ol className="mt-3 space-y-3">
            {view.journal.map(entry => (
              <li key={entry.turn_id} className="rounded-lg border border-[#d2e3fc] bg-[#f8faff] p-3 text-sm text-[#3c4043]">
                <span className="mr-2 text-xs font-semibold text-[#0b57d0]">回合 {entry.turn_number}</span>
                <span className="mr-2 text-xs font-semibold text-[#0d652d]">读者行动</span>
                {actionLabels[entry.action.kind]}：{entry.action.intent}
                <div className="mt-1 text-xs text-[#5f6368]">
                  <span className="mr-2 font-semibold text-[#0b57d0]">生成叙事</span>
                  {entry.transition.rendered_narrative}
                </div>
                <time dateTime={entry.completed_at} className="mt-1 block text-xs text-[#5f6368]">
                  {entry.completed_at}
                </time>
              </li>
            ))}
          </ol>
        ) : <p className="mt-3 text-sm text-[#5f6368]">你的第一个行动将记录在这里。</p>}
      </div>

      <div className="border-t border-[#e1e3e8] pt-6">
        <h3 className="mb-4 text-sm font-semibold text-[#1f1f1f]">采取下一步行动</h3>
        <WorldActionForm
          view={view}
          isPending={turn.isPending}
          isLocked={Boolean(pendingRequest)}
          onSubmit={submit}
        />
        {error ? (
          <div role="alert" className="mt-4 text-sm text-[#b3261e]">
            {error}
            {pendingRequest ? (
              <button className="ml-2 underline" disabled={turn.isPending} onClick={() => void run(pendingRequest).catch(() => undefined)}>
                使用同一个请求重试
              </button>
            ) : null}
            {pendingRequest ? (
              <button className="ml-2 underline" disabled={turn.isPending} onClick={() => {
                setPendingRequest(null);
                setError(undefined);
              }}>
                放弃此请求
              </button>
            ) : null}
          </div>
        ) : null}
      </div>
    </section>
  );
}
