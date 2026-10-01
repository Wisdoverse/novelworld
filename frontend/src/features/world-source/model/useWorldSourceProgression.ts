import { useEffect, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { advanceReadingProgress, fetchReadingProgress, readingProgressKeys } from '@/entities/reading-progress';
import { advanceWorldSource, fetchOpenWorld, narrativeKeys } from '@/entities/narrative';
import { getApiErrorCode, getApiErrorMessage } from '@/shared/api/client';
import { effectiveWorldContext } from '@/shared/lib/worldSourceContext';
import { worldTurnPendingStorageKey } from '@/shared/lib/worldTurnStorage';
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
  const finishFlight = () => {
    flight.current = false;
    // Wake the current book and retire the old book's busy status on navigation.
    // Preserve recovery keys and never publish an old scope's private response.
    if (mounted.current) setStatus(current => ({ ...current, pending: false }));
  };
  const remember = (next: PendingWorldSource | null) => {
    if (!mounted.current || !progress?.user_id || currentScope.current !== scope) return;
    storePendingWorldSource(progress.user_id, novelId, next);
    setState({ scope, pending: next });
  };
  useEffect(() => {
    if (pending?.notDispatched === true && progress?.reader_identity_type === 'character') {
      // This new automatic source request was never dispatched. Keep
      // character-mode original reading usable without dropping unknown outcomes.
      if (routeChapter !== progress.current_chapter) {
        navigate(`/reader/${novelId}/${progress.current_chapter}`);
        return;
      }
      remember(null);
      return;
    }
    if (pending?.synchronizedChapter && routeChapter !== undefined && routeChapter === progress?.current_chapter
      && routeChapter >= pending.synchronizedChapter) remember(null);
  }, [pending, routeChapter, progress?.current_chapter, progress?.reader_identity_type, scope]);

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

  const run = async (operation: PendingWorldSource, automatic = false) => {
    if (flight.current || !progress?.user_id) return;
    flight.current = true;
    setIsPending(true);
    setError(undefined);
    remember({ ...operation, notDispatched: undefined });
    let confirmed = operation;
    let sourceRequested = false;
    try {
      const fresh = await fetchReadingProgress(novelId);
      if (!mounted.current || currentScope.current !== scope) return;
      if (fresh.user_id !== progress.user_id || fresh.novel_id !== novelId
        || fresh.reader_identity_type !== 'self' || fresh.current_chapter < operation.request.expected_source_chapter
        || (automatic && fresh.current_chapter < progress.current_chapter)) {
        remember({ ...operation, terminal: automatic, notDispatched: automatic });
        if (fresh.user_id === progress.user_id && fresh.novel_id === novelId) {
          queryClient.setQueryData(readingProgressKeys.detail(novelId), fresh);
        }
        setError('阅读身份或进度已变化，自动接续已暂停。请恢复最新状态。');
        return;
      }
      // A repeated monotonic unlock is safe after either owner's response is lost.
      // The owner atomically checks this fresh snapshot and self identity so a
      // concurrent rewind or identity switch cannot be overwritten by this call.
      await advanceReadingProgress(novelId, operation.request.target_chapter, fresh.current_chapter);
      if (!mounted.current || currentScope.current !== scope) return;
      sourceRequested = true;
      const result = await advanceWorldSource(novelId, operation.request, operation.idempotencyKey);
      confirmed = { ...operation, result: { operation_id: result.operation_id,
        previous_source_chapter: result.previous_source_chapter, source_chapter: result.source_chapter } };
      remember(confirmed);
      await synchronize(confirmed, result.view, result.source_chapter);
    } catch (failure) {
      if (!mounted.current || currentScope.current !== scope) return;
      const code = getApiErrorCode(failure);
      if (code === 'reading_progress_changed' || code === 'reader_identity_unavailable') {
        // A progress refusal cannot settle an older unknown source outcome.
        const notDispatched = automatic && !sourceRequested;
        remember({ ...confirmed, terminal: notDispatched, notDispatched });
      }
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
      setError(code === 'reading_progress_changed' || code === 'reader_identity_unavailable'
        ? '阅读身份或进度已变化，自动接续已暂停。请恢复最新状态。'
        : code === 'world_source_busy'
        ? '上一行动还在确认，下一幕暂时不能接入。请稍后重试原请求。'
        : code === 'world_source_changed'
          ? '另一窗口已经改变世界。请恢复最新世界，再决定下一步。'
          : code === 'world_source_order_conflict'
            ? '新来源的事件顺序与已经发生的剧情冲突，暂时不能接入。'
            : code === 'world_source_unavailable'
              ? '下一章暂时没有可接入的完整来源，请恢复最新世界。'
              : getApiErrorMessage(failure, '下一章可能已解锁，但世界接入结果尚未确认；请继续确认原请求。'));
    } finally {
      finishFlight();
    }
  };
  const start = (view: OpenWorldView, automatic = false) => {
    if (pending || flight.current) return;
    const source = effectiveWorldContext(view.session).unlocked_through_chapter;
    void run({ idempotencyKey: crypto.randomUUID(), request: {
      expected_turn_number: view.session.turn_number,
      expected_source_chapter: source,
      target_chapter: source + 1,
    } }, automatic);
  };
  const advanceIfReady = (view: OpenWorldView, totalChapters: number) => {
    if (!progress || progress.reader_identity_type !== 'self'
      || view.player?.user_id !== progress.user_id || view.player.novel_id !== novelId
      || routeChapter !== progress.current_chapter || pending || flight.current) return;
    const source = effectiveWorldContext(view.session).unlocked_through_chapter;
    const latest = view.journal.find(entry => entry.turn_number === view.session.turn_number);
    // The committed turn's source is the durable fence: admitting a chapter
    // cannot make that same turn admit another one, including after reload.
    const turnSource = latest?.expected_source_chapter ?? view.session.entry_context.unlocked_through_chapter;
    if (!latest || turnSource !== source || source >= totalChapters || progress.current_chapter < source
      || view.recoverable_turn || view.journal.some(entry => entry.memory_projection_status === 'pending')
      || (latest.memory_projection_status !== 'saved' && latest.memory_projection_status !== 'skipped')
      || view.session.canonical_events.some(event => event.status === 'scheduled' || event.status === 'delayed')) return;
    try {
      if (window.sessionStorage.getItem(worldTurnPendingStorageKey(progress.user_id, novelId))) return;
    } catch { /* The current dashboard lock protects restricted-storage mounts. */ }
    start(view, true);
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
    } finally { finishFlight(); }
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
      finishFlight();
    }
  };
  return { continueOriginalReading, locked: Boolean(pending) || isPending, pending, isPending, error, start, advanceIfReady, recover };
}
