import { useEffect, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { advanceReadingProgress, fetchReadingProgress, readingProgressKeys } from '@/entities/reading-progress';
import { advanceWorldSource, fetchOpenWorld, narrativeKeys } from '@/entities/narrative';
import { getApiErrorCode, getApiErrorMessage } from '@/shared/api/client';
import { effectiveWorldContext } from '@/shared/lib/worldSourceContext';
import { readPendingWorldSource, storePendingWorldSource, type PendingWorldSource } from '@/shared/lib/worldSourceStorage';
import type { OpenWorldView, ReadingProgress } from '@/shared/types';

export function useWorldSourceProgression({ novelId, progress, routeChapter, navigate }: {
  novelId: string;
  progress?: ReadingProgress;
  routeChapter?: number;
  navigate: (path: string) => void;
}) {
  const queryClient = useQueryClient();
  const scope = `${progress?.user_id}:${novelId}`;
  const [state, setState] = useState<{ scope: string; pending: PendingWorldSource | null }>(() => ({
    scope, pending: readPendingWorldSource(progress?.user_id, novelId),
  }));
  // Resolve storage synchronously before ReaderPage's absolute progress-save effect,
  // including the render where the authenticated progress first becomes available.
  const pending = state.scope === scope ? state.pending : readPendingWorldSource(progress?.user_id, novelId);
  const [status, setStatus] = useState<{ scope: string; pending: boolean; error?: string }>({ scope, pending: false });
  const isPending = status.scope === scope && status.pending;
  const error = status.scope === scope ? status.error : undefined;
  const setIsPending = (value: boolean) => setStatus(current => ({
    ...(current.scope === scope ? current : { scope }), pending: value,
  }));
  const setError = (message?: string) => setStatus(current => ({
    ...(current.scope === scope ? current : { scope, pending: false }), error: message,
  }));
  const flight = useRef(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  const currentScope = useRef(scope);
  currentScope.current = scope;
  const remember = (next: PendingWorldSource | null) => {
    if (!mounted.current || !progress?.user_id || currentScope.current !== scope) return;
    storePendingWorldSource(progress.user_id, novelId, next);
    setState({ scope, pending: next });
  };
  useEffect(() => {
    if (pending?.synchronizedChapter && routeChapter !== undefined && routeChapter === progress?.current_chapter
      && routeChapter >= pending.synchronizedChapter) remember(null);
  }, [pending, routeChapter, progress?.current_chapter, scope]);

  const synchronize = async (operation: PendingWorldSource, view: OpenWorldView, admittedChapter: number) => {
    if (!mounted.current || currentScope.current !== scope) return;
    const freshProgress = await fetchReadingProgress(novelId);
    if (!mounted.current || currentScope.current !== scope || freshProgress.user_id !== progress?.user_id
      || freshProgress.reader_identity_type !== 'self') throw new Error('阅读身份已变化，请重新加载。');
    const source = effectiveWorldContext(view.session).unlocked_through_chapter;
    // A rewind after commit hides source content. Keep the exact key until the
    // reader explicitly restores progress; do not silently undo the rewind.
    if (freshProgress.current_chapter < source) throw new Error('阅读进度低于当前世界来源；请恢复进度后继续确认。');
    const chapter = Math.max(source, admittedChapter, freshProgress.current_chapter);
    remember({ ...operation, synchronizedChapter: chapter });
    queryClient.setQueryData(readingProgressKeys.detail(novelId), freshProgress);
    queryClient.setQueryData(narrativeKeys.openWorld(novelId), view);
    queryClient.setQueryData(narrativeKeys.worldState(novelId), view.world_state);
    navigate(`/reader/${novelId}/${chapter}#latest-world-narrative`);
  };

  const run = async (operation: PendingWorldSource) => {
    if (flight.current || !progress?.user_id) return;
    flight.current = true;
    setIsPending(true);
    setError(undefined);
    remember(operation);
    let confirmed = operation;
    try {
      // A repeated monotonic unlock is safe after either owner's response is lost.
      await advanceReadingProgress(novelId, operation.request.target_chapter);
      if (!mounted.current || currentScope.current !== scope) return;
      const result = await advanceWorldSource(novelId, operation.request, operation.idempotencyKey);
      confirmed = { ...operation, result: { operation_id: result.operation_id,
        previous_source_chapter: result.previous_source_chapter, source_chapter: result.source_chapter } };
      remember(confirmed);
      await synchronize(confirmed, result.view, result.source_chapter);
    } catch (failure) {
      if (!mounted.current || currentScope.current !== scope) return;
      const code = getApiErrorCode(failure);
      if (code === 'world_source_changed' || code === 'world_source_order_conflict'
        || code === 'world_source_unavailable' || code === 'invalid_request'
        || code === 'idempotency_conflict') remember({ ...confirmed, terminal: true });
      if (code === 'world_source_busy') {
        // The unresolved turn can belong to a closed tab. Keep this source key,
        // but recover the server's exact turn instead of waiting forever.
        try {
          const view = await fetchOpenWorld(novelId);
          if (!mounted.current || currentScope.current !== scope) return;
          queryClient.setQueryData(narrativeKeys.openWorld(novelId), view);
          queryClient.setQueryData(narrativeKeys.worldState(novelId), view.world_state);
        } catch {
          // A failed guarded read keeps both mutations locked; retry can refetch.
        }
      }
      setError(code === 'world_source_busy'
        ? '上一行动还在确认，下一幕暂时不能接入。请稍后重试原请求。'
        : code === 'world_source_changed'
          ? '另一窗口已经改变世界。请恢复最新世界，再决定下一步。'
          : code === 'world_source_order_conflict'
            ? '新来源的事件顺序与已经发生的剧情冲突，暂时不能接入。'
            : code === 'world_source_unavailable'
              ? '下一章暂时没有可接入的完整来源，请恢复最新世界。'
              : getApiErrorMessage(failure, '下一章可能已解锁，但世界接入结果尚未确认；请继续确认原请求。'));
    } finally {
      flight.current = false;
      if (mounted.current && currentScope.current === scope) setIsPending(false);
    }
  };
  const start = (view: OpenWorldView) => {
    if (pending || flight.current) return;
    const source = effectiveWorldContext(view.session).unlocked_through_chapter;
    void run({ idempotencyKey: crypto.randomUUID(), request: {
      expected_turn_number: view.session.turn_number,
      expected_source_chapter: source,
      target_chapter: source + 1,
    } });
  };
  const recover = async () => {
    if (!pending || flight.current) return;
    if (!pending.terminal) { await run(pending); return; }
    flight.current = true;
    setIsPending(true);
    try {
      const view = await fetchOpenWorld(novelId);
      await synchronize(pending, view, effectiveWorldContext(view.session).unlocked_through_chapter);
      setError(undefined);
    } catch (failure) {
      if (mounted.current && currentScope.current === scope) setError(getApiErrorMessage(failure, '最新世界尚未恢复，请稍后重试。'));
    } finally { flight.current = false; if (mounted.current && currentScope.current === scope) setIsPending(false); }
  };
  const continueOriginalReading = async (totalChapters: number) => {
    if (!pending || flight.current || !progress?.user_id) return;
    flight.current = true; setIsPending(true); setError(undefined);
    try {
      const fresh = await fetchReadingProgress(novelId);
      if (!mounted.current || currentScope.current !== scope || fresh.user_id !== progress.user_id) return;
      const next = Math.max(routeChapter ?? fresh.current_chapter, fresh.current_chapter) + 1;
      if (next > totalChapters) throw new Error('已经读到原著最后一章，请继续确认世界接入结果。');
      const advanced = await advanceReadingProgress(novelId, next);
      if (!mounted.current || currentScope.current !== scope || advanced.user_id !== progress.user_id) return;
      queryClient.setQueryData(readingProgressKeys.detail(novelId), advanced);
      navigate(`/reader/${novelId}/${advanced.current_chapter}`);
      // Keep the unresolved source identity. Reading the original source never
      // fabricates another admission or abandons a possibly committed operation.
    } catch (failure) {
      if (mounted.current && currentScope.current === scope) setError(getApiErrorMessage(failure, '原著阅读进度暂时无法恢复，请重试。'));
    } finally {
      flight.current = false;
      if (mounted.current && currentScope.current === scope) setIsPending(false);
    }
  };
  return { continueOriginalReading, locked: Boolean(pending) || isPending, pending, isPending, error, start, recover };
}
