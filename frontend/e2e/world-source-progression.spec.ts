import { test, expect, type Page } from '@playwright/test';
import type { WorldTurnJournalEntry } from '../src/shared/types';
import { installStubs } from './stubs';
import { OPEN_WORLD, PROGRESS, JOURNAL_ENTRY } from './fixtures';
import { expectNoA11yViolations, settleAnimations } from './helpers';

function trackWorldTurnRecoveryRequests(page: Page) {
  const counts = { confirmations: 0, actions: 0 };
  page.on('request', request => {
    const path = new URL(request.url()).pathname.replace(/^\/api/, '');
    if (request.method() === 'GET' && /^\/narrative\/[^/]+\/world\/turns\/[^/]+$/.test(path)) counts.confirmations++;
    if (request.method() === 'POST' && path === '/narrative/novel-1/world/turns') counts.actions++;
  });
  return counts;
}

async function confirmReadOnlyThenResume(page: Page, requests: { confirmations: number; actions: number }) {
  const readsBefore = requests.confirmations;
  await page.getByRole('button', { name: '继续确认结果', exact: true }).click();
  await expect.poll(() => requests.confirmations).toBeGreaterThan(readsBefore);
  expect(requests.actions).toBe(0);
  await page.getByRole('button', { name: '恢复原行动', exact: true }).click();
  await expect.poll(() => requests.actions).toBe(1);
}

async function sourceWorld(page: Page, options: { loseResponse?: boolean; busy?: boolean; progressAfter?: number; failProgress?: boolean; stale?: boolean; end?: boolean; empty?: boolean; currentSourceAfter?: number; rewindAfter?: number; freshCharacterChapter?: number; busyRace?: 'in_progress' | 'pending_projection' } = {}) {
  await installStubs(page, { openWorld: true });
  let progress = options.freshCharacterChapter ? 3 : options.end ? 5 : 1;
  let source = options.end ? 5 : 1;
  let turn = 1;
  let sourceRequests = 0;
  let progressReads = 0;
  const sourceCommands: Array<{ key: string | undefined; body: unknown }> = [];
  const absoluteWrites: number[] = [];
  const actions: Array<Record<string, unknown>> = [];
  const actionKeys: Array<string | undefined> = [];
  const sourceOperations = new Set<string>();
  const recoveryKey = 'a733c562-44d3-4ae0-83b8-33a28c7be35a';
  const recoveryAction = { kind: 'travel' as const, target_id: 'loc-2', intent: '确认上一窗口已经提交的旅程' };
  let recoverable: { turn_id: string; action: typeof recoveryAction; expected_turn_number: number; expected_source_chapter: number } | undefined;
  let pendingProjection = false;
  let providerCalls = 0;
  const nextEvent = {
    id: 'next-scene-event', sequence: 2, summary: '第二幕的商船靠岸', character_ids: [],
    location_ids: ['loc-2'], faction_ids: [], death_character_ids: [], source_chapters: [2],
  };
  let journal = structuredClone(OPEN_WORLD.journal).map(entry => ({ ...entry, expected_source_chapter: 1 })) as WorldTurnJournalEntry[];
  const view = () => ({
    ...OPEN_WORLD,
    session: {
      ...OPEN_WORLD.session,
      turn_number: turn,
      canonical_events: source >= 2 && !options.empty ? [{ ...nextEvent, status: turn > 1 ? 'witnessed' : 'scheduled', reason: null }] : [],
      ...(source >= 2 ? { schema_version: 2, source_context: {
        ...OPEN_WORLD.session.entry_context, unlocked_through_chapter: source,
        scheduled_events: options.empty ? [] : [nextEvent], locations: [...OPEN_WORLD.session.entry_context.locations, { id: 'dock-2', name: '第二幕的新码头' }],
      } } : {}),
    },
    journal,
    recoverable_turn: recoverable,
  });
  await page.route('**/api/**', async route => {
    const req = route.request();
    const path = new URL(req.url()).pathname.replace(/^\/api/, '');
    const json = (body: unknown, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
    if (path === '/progress/novel-1') {
      if (req.method() === 'PUT') { progress = req.postDataJSON().current_chapter; absoluteWrites.push(progress); }
      progressReads++;
      if (options.freshCharacterChapter && progressReads > 1) {
        progress = options.freshCharacterChapter;
        return json({ ...PROGRESS, current_chapter: progress, reader_identity_type: 'character', reader_character_id: 'char-1', reader_identity: '沈知微' });
      }
      if (options.failProgress && sourceRequests > 0) return json({ error: { code: 'temporary_unavailable', message: '暂时无法恢复进度' } }, 503);
      return json({ ...PROGRESS, current_chapter: progress });
    }
    if (path === '/progress/novel-1/advance') {
      progress = Math.max(progress, req.postDataJSON().current_chapter);
      return json({ ...PROGRESS, current_chapter: progress });
    }
    if (path === '/narrative/novel-1/world/source') {
      sourceRequests++;
      sourceCommands.push({ key: req.headers()['idempotency-key'], body: req.postDataJSON() });
      if (options.stale) { source = 2; return json({ error: { code: 'world_source_changed', message: 'stale' } }, 409); }
      if ((options.busy || options.busyRace) && sourceRequests === 1) {
        if (options.busyRace === 'in_progress') recoverable = { turn_id: recoveryKey, action: recoveryAction, expected_turn_number: turn, expected_source_chapter: source };
        if (options.busyRace === 'pending_projection') {
          pendingProjection = true;
          journal = [{ ...journal[0], turn_id: recoveryKey, action: recoveryAction, expected_source_chapter: source, memory_projection_status: 'pending' }] as typeof journal;
        }
        return json({ error: { code: 'world_source_busy', message: 'busy' } }, 409);
      }
      const key = req.headers()['idempotency-key'];
      if (!sourceOperations.has(key) && req.postDataJSON().expected_turn_number !== turn) return json({ error: { code: 'world_source_changed', message: 'stale' } }, 409);
      sourceOperations.add(key);
      source = Math.max(source, options.currentSourceAfter ?? req.postDataJSON().target_chapter);
      if (options.rewindAfter && sourceRequests === 1) progress = options.rewindAfter;
      if (options.progressAfter) progress = options.progressAfter;
      if (options.loseResponse && sourceRequests === 1) return route.abort('failed');
      return json({ operation_id: req.headers()['idempotency-key'], previous_source_chapter: req.postDataJSON().expected_source_chapter, source_chapter: req.postDataJSON().target_chapter, view: view() });
    }
    if (path === '/narrative/novel-1/world') return json(view());
    if (path === '/narrative/novel-1/world-state') return json(view().world_state);
    if (path === '/narrative/novel-1/world/turns') {
      const command = req.postDataJSON(); actions.push(command); actionKeys.push(req.headers()['idempotency-key']);
      if (options.busyRace && req.headers()['idempotency-key'] !== recoveryKey) return json({ error: { code: 'turn_conflict', message: 'must replay the original turn' } }, 409);
      if (pendingProjection) {
        pendingProjection = false;
        journal = journal.map(entry => ({ ...entry, memory_projection_status: 'saved' }));
        return json({ ...journal[0], memory_projection_status: 'saved', world_state: view().world_state });
      }
      recoverable = undefined;
      providerCalls++;
      if (command.expected_source_chapter !== source) return json({ error: { code: 'world_source_changed', message: 'stale' } }, 409);
      turn++;
      const entry = { ...JOURNAL_ENTRY, turn_id: req.headers()['idempotency-key'], turn_number: turn, expected_source_chapter: command.expected_source_chapter,
        action: { kind: command.kind, target_id: command.target_id, intent: command.intent },
        transition: { ...JOURNAL_ENTRY.transition, rendered_narrative: '你见证商船靠岸，第二幕已展开。',
          canonical_event_change: { event_id: nextEvent.id, status: 'witnessed', reason: '读者调查' } },
      };
      journal = [...journal, entry] as typeof journal;
      return json({ ...entry, memory_projection_status: 'saved', world_state: view().world_state });
    }
    if (/^\/novels\/novel-1\/chapters\/\d+$/.test(path)) {
      return json({ chapter_number: Number(path.split('/').pop()), title: '原著参考', content: '不可变的原著正文', is_key_node: false });
    }
    return route.fallback();
  });
  return {
    sourceCommands, absoluteWrites, actions, actionKeys, recoveryKey, recoveryAction,
    get providerCalls() { return providerCalls; },
    get source() { return source; }, get progressReads() { return progressReads; },
    rewind() { progress = 1; }, staleAdvance() { source = 2; },
    allowProgress() { options.failProgress = false; },
  };
}

test('turn clock automatically admits chapters 2 and 3 without a scene click', async ({ page }) => {
  const server = await sourceWorld(page);
  await page.goto('/reader/novel-1/1#world-action-form');
  await expect(page.getByRole('button', { name: '进入下一幕', exact: true })).toHaveCount(0);
  await expect(page).toHaveURL(/\/reader\/novel-1\/2#latest-world-narrative$/);
  await expect(page.locator('li').filter({ hasText: '第二幕的商船靠岸' })).toBeVisible();
  await expect(page.getByText('当前原著进度 · 第 2 章')).toBeVisible();
  await expect(page.getByText('长行动投影起点', { exact: false })).toBeVisible();
  expect(server.absoluteWrites).toEqual([]);
  expect(server.actions).toEqual([]);
  expect(server.providerCalls).toBe(0);
  expect(server.sourceCommands[0].body).toEqual({ expected_turn_number: 1, expected_source_chapter: 1, target_chapter: 2 });
  await page.locator('summary').filter({ hasText: '调整行动方式与目标' }).click();
  await page.getByRole('combobox', { name: '行动', exact: true }).selectOption('investigate');
  await page.getByRole('combobox', { name: '目标', exact: true }).selectOption('next-scene-event');
  await page.getByLabel('你的意图').fill('观察商船靠岸');
  await page.getByRole('button', { name: '执行行动', exact: true }).click();
  await expect(page.locator('#latest-world-narrative')).toContainText('第二幕已展开');
  await expect(page).toHaveURL(/\/reader\/novel-1\/3#latest-world-narrative$/);
  expect(server.actions[0].expected_source_chapter).toBe(2);
  expect(server.sourceCommands).toHaveLength(2);
  expect(server.sourceCommands[1].body).toEqual({ expected_turn_number: 2, expected_source_chapter: 2, target_chapter: 3 });
  expect(server.providerCalls).toBe(1);
  await page.reload();
  await expect(page.getByRole('button', { name: '执行行动', exact: true })).toBeVisible();
  expect(server.sourceCommands).toHaveLength(2);
  await settleAnimations(page);
  await expectNoA11yViolations(page);
  await page.getByRole('region', { name: '世界来源进度' }).screenshot({ path: '/tmp/novelworld-world-source-progression-ui.png' });
});

test('a fresh character identity and rewind keep original reading usable without a stale progress write', async ({ page }) => {
  const server = await sourceWorld(page, { freshCharacterChapter: 1 });
  await page.goto('/reader/novel-1/3');
  await expect(page).toHaveURL(/\/reader\/novel-1\/1$/);
  await expect(page.getByRole('button', { name: '下一章', exact: true })).toBeEnabled();
  expect(server.sourceCommands).toEqual([]);
  expect(server.absoluteWrites).toEqual([]);
  expect(server.providerCalls).toBe(0);
  await page.reload();
  await expect(page.getByRole('button', { name: '下一章', exact: true })).toBeEnabled();
  expect(server.sourceCommands).toEqual([]);
  expect(server.absoluteWrites).toEqual([]);
});

test('lost response survives remount with the exact key and never rewinds progress', async ({ page }) => {
  const server = await sourceWorld(page, { loseResponse: true });
  await page.goto('/reader/novel-1/1');
  await expect(page.getByRole('button', { name: '继续确认下一幕' })).toBeEnabled();
  await page.reload();
  await page.getByRole('button', { name: '继续确认下一幕' }).click();
  await expect(page).toHaveURL(/\/reader\/novel-1\/2#latest-world-narrative$/);
  expect(server.sourceCommands).toHaveLength(2);
  expect(server.sourceCommands[1]).toEqual(server.sourceCommands[0]);
  expect(server.absoluteWrites).toEqual([]);
});

test('replay admits 2 but synchronizes a concurrent authoritative progress 3', async ({ page }) => {
  const server = await sourceWorld(page, { loseResponse: true, progressAfter: 3 });
  await page.goto('/reader/novel-1/1');
  await expect(page.getByRole('button', { name: '继续确认下一幕' })).toBeEnabled();
  await page.reload();
  await page.getByRole('button', { name: '继续确认下一幕' }).click();
  await expect(page).toHaveURL(/\/reader\/novel-1\/3#latest-world-narrative$/);
  await page.locator('summary').filter({ hasText: '调整行动方式与目标' }).click();
  await expect(page.getByRole('combobox', { name: '行动', exact: true })).toBeEnabled();
  expect(server.absoluteWrites).toEqual([]);
  expect(server.sourceCommands[1]).toEqual(server.sourceCommands[0]);
});

test('monotonic unlock followed by busy source retains key and fences absolute PUT on old route', async ({ page }) => {
  const server = await sourceWorld(page, { busy: true });
  await page.goto('/reader/novel-1/1');
  await expect(page.getByRole('alert')).toContainText('上一行动还在确认');
  await page.reload();
  await expect(page.getByRole('button', { name: '继续确认下一幕' })).toBeEnabled();
  expect(server.absoluteWrites).toEqual([]);
  await page.getByRole('button', { name: '继续确认下一幕' }).click();
  await expect(page).toHaveURL(/\/reader\/novel-1\/2#latest-world-narrative$/);
  expect(server.sourceCommands[1]).toEqual(server.sourceCommands[0]);
  expect(server.absoluteWrites).toEqual([]);
});

test('failed postcommit progress confirmation locks until exact replay and fresh progress recovery', async ({ page }) => {
  const server = await sourceWorld(page, { failProgress: true });
  await page.goto('/reader/novel-1/1');
  await expect(page.getByRole('button', { name: '继续确认下一幕' })).toBeEnabled();
  await expect(page.getByRole('button', { name: '下一章', exact: true })).toBeDisabled();
  expect(server.absoluteWrites).toEqual([]);
  server.allowProgress();
  await page.getByRole('button', { name: '继续确认下一幕' }).click();
  await expect(page).toHaveURL(/\/reader\/novel-1\/2#latest-world-narrative$/);
  expect(server.sourceCommands[1]).toEqual(server.sourceCommands[0]);
});

test('deliberate rewind hides active source event, catalogs and chat before refetch', async ({ page }) => {
  const server = await sourceWorld(page);
  await page.goto('/reader/novel-1/1');
  await expect(page).toHaveURL(/\/reader\/novel-1\/2#latest-world-narrative$/);
  // Navigate through the router's source path. Absolute progress PUT remains
  // the intentional rewind contract once source recovery has finished.
  await page.evaluate(() => { window.history.pushState(null, '', '/reader/novel-1/1'); window.dispatchEvent(new PopStateEvent('popstate')); });
  await expect(page.locator('li').filter({ hasText: '第二幕的商船靠岸' })).toHaveCount(0);
  await expect(page.getByText('第二幕的新码头', { exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: '进入下一幕', exact: true })).toHaveCount(0);
  await expect.poll(() => server.absoluteWrites).toEqual([1]);
});


test('stale source operation restores current world without issuing a new command', async ({ page }) => {
  const server = await sourceWorld(page, { stale: true });
  await page.goto('/reader/novel-1/1');
  await expect(page.getByRole('alert')).toContainText('另一窗口已经改变世界');
  await page.getByRole('button', { name: '恢复最新世界', exact: true }).click();
  await expect(page).toHaveURL(/\/reader\/novel-1\/2#latest-world-narrative$/);
  await page.locator('summary').filter({ hasText: '调整行动方式与目标' }).click();
  await expect(page.getByRole('combobox', { name: '行动', exact: true })).toBeEnabled();
  expect(server.sourceCommands).toHaveLength(1);
  expect(server.absoluteWrites).toEqual([]);
});

test('source end and empty extraction explain the actual available progression', async ({ page }) => {
  await sourceWorld(page, { end: true, empty: true });
  await page.goto('/reader/novel-1/5');
  await expect(page.getByText('已接入原著最后一章。你仍可在当前世界行动。')).toBeVisible();
  await expect(page.getByRole('button', { name: '进入下一幕', exact: true })).toHaveCount(0);
  await expect(page.getByText('当前解锁范围内没有待运行的原著事件。')).toBeVisible();
  await page.locator('summary').filter({ hasText: '调整行动方式与目标' }).click();
  await expect(page.getByRole('combobox', { name: '行动', exact: true })).toBeEnabled();
});


test('unknown source replay survives a later source and rewind by explicit original reading', async ({ page }) => {
  const server = await sourceWorld(page, { loseResponse: true, currentSourceAfter: 3, rewindAfter: 1 });
  await page.goto('/reader/novel-1/1');
  await expect(page.getByRole('button', { name: '继续确认下一幕' })).toBeEnabled();
  await page.getByRole('button', { name: '继续阅读原文下一章' }).click();
  await expect(page).toHaveURL(/\/reader\/novel-1\/2$/);
  await expect(page.getByRole('combobox', { name: '行动', exact: true }).and(page.locator(':enabled'))).toHaveCount(0);
  await expect(page.getByRole('button', { name: '自由输入', exact: true }).and(page.locator(':enabled'))).toHaveCount(0);
  await expect(page.getByRole('group', { name: '场景建议' }).getByRole('button').and(page.locator(':enabled'))).toHaveCount(0);
  await expect(page.getByRole('button', { name: '执行行动', exact: true }).and(page.locator(':enabled'))).toHaveCount(0);
  await page.getByRole('button', { name: '继续阅读原文下一章' }).click();
  await expect(page).toHaveURL(/\/reader\/novel-1\/3$/);
  await page.getByRole('button', { name: '继续确认下一幕' }).click();
  await expect(page).toHaveURL(/\/reader\/novel-1\/3#latest-world-narrative$/);
  await page.locator('summary').filter({ hasText: '调整行动方式与目标' }).click();
  await expect(page.getByRole('combobox', { name: '行动', exact: true })).toBeEnabled();
  expect(server.sourceCommands[1]).toEqual(server.sourceCommands[0]);
  expect(server.absoluteWrites).toEqual([]);
});


test('busy source restores the exact pending memory turn while keeping new actions locked', async ({ page }) => {
  const requests = trackWorldTurnRecoveryRequests(page);
  const server = await sourceWorld(page, { busyRace: 'pending_projection' });
  await page.goto('/reader/novel-1/1');
  await expect(page.getByRole('button', { name: '继续确认结果', exact: true })).toBeEnabled();
  await page.locator('summary').filter({ hasText: '调整行动方式与目标' }).click();
  await expect(page.getByRole('combobox', { name: '行动', exact: true })).toBeDisabled();
  await expect(page.getByRole('button', { name: '执行行动', exact: true })).toBeDisabled();
  await page.reload();
  // Persisted progress is now 2 while the route/source remain 1. Only the
  // authoritative original turn may recover through this deliberate mismatch.
  await confirmReadOnlyThenResume(page, requests);
  await expect(page.getByRole('button', { name: '继续确认结果', exact: true })).toHaveCount(0);
  await page.locator('summary').filter({ hasText: '调整行动方式与目标' }).click();
  await expect(page.getByRole('combobox', { name: '行动', exact: true })).toBeDisabled();
  await page.getByRole('button', { name: '继续确认下一幕' }).click();
  await expect(page).toHaveURL(/\/reader\/novel-1\/2#latest-world-narrative$/);
  expect(server.actions).toEqual([{ ...server.recoveryAction, expected_turn_number: 0, expected_source_chapter: 1 }]);
  expect(server.actionKeys).toEqual([server.recoveryKey]);
  expect(server.sourceCommands[1]).toEqual(server.sourceCommands[0]);
  expect(server.providerCalls).toBe(0);
  expect(server.absoluteWrites).toEqual([]);
});

test('in-progress original turn recovery preserves the source fence and automatically retries admission from the recovered clock', async ({ page }) => {
  const requests = trackWorldTurnRecoveryRequests(page);
  const server = await sourceWorld(page, { busyRace: 'in_progress' });
  await page.goto('/reader/novel-1/1');
  await expect(page.getByRole('button', { name: '继续确认结果', exact: true })).toBeEnabled();
  await page.reload();
  await confirmReadOnlyThenResume(page, requests);
  await expect(page.getByRole('button', { name: '继续确认结果', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: '执行行动', exact: true })).toBeDisabled();
  await page.getByRole('button', { name: '继续确认下一幕' }).click();
  await expect(page.getByRole('button', { name: '恢复最新世界', exact: true })).toBeEnabled();
  expect(server.sourceCommands[1]).toEqual(server.sourceCommands[0]);
  await page.getByRole('button', { name: '恢复最新世界', exact: true }).click();
  await expect(page.locator('li').filter({ hasText: '第二幕的商船靠岸' })).toBeVisible();
  expect(server.actions).toEqual([{ ...server.recoveryAction, expected_turn_number: 1, expected_source_chapter: 1 }]);
  expect(server.actionKeys).toEqual([server.recoveryKey]);
  expect(server.sourceCommands[2].key).not.toBe(server.sourceCommands[0].key);
  expect(server.sourceCommands[2].body).toEqual({ expected_turn_number: 2, expected_source_chapter: 1, target_chapter: 2 });
  expect(server.providerCalls).toBe(1);
  expect(server.absoluteWrites).toEqual([]);
});

// These established journeys intentionally exercise the Chinese UI.
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('novelworld.ui.locale', 'zh-CN'));
});
