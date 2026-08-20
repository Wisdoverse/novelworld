import type { Page, Request } from '@playwright/test';
import {
  AUTH_TOKENS, CHAPTER, CHARACTERS, CHOICE_RESULT, EFFECTIVE_CHAPTER, JOURNAL_ENTRY,
  LLM_SETTINGS, NODE, NOVEL, NOVELS, OPEN_WORLD, PLAYER_ENTRY, PLAYER_ENTRY_NO_PLAYER,
  PROGRESS, SETUP_STATUS, WORLD_STATE, WORLD_TURN_RESULT, USER,
} from './fixtures';

export interface StubOptions {
  /** /setup/status reports configured:false so the app boots into SetupPage. */
  setupNeeded?: boolean;
  /** Open-world view present on the reader page (WorldDashboard + WorldActionForm render). */
  openWorld?: boolean;
  /** Player-entry has no player yet (PlayerEntryForm renders). */
  entryRequired?: boolean;
}

const json = (status: number, body: unknown) => ({
  status,
  contentType: 'application/json',
  body: JSON.stringify(body),
});

const sse = (body: string) => ({
  status: 200,
  contentType: 'text/event-stream',
  body,
});

// One committed legacy 'done' frame — the parser treats it as a completed turn.
const CHAT_STREAM = sse('event: done\n\n');

const CHAT_DELTA_STREAM = (turnId: string) => sse(
  'event: delta\ndata: {"content":"星光洒在海面上。"}\n\n'
  + 'event: done\ndata: {"turn_id":"' + turnId + '","committed":true,"replayed":false}\n\n',
);

/** Route table: method + pathname regex -> response builder. */
export async function installStubs(page: Page, opts: StubOptions = {}): Promise<void> {
  const openWorld = opts.openWorld ?? false;
  const entry = opts.entryRequired ? PLAYER_ENTRY_NO_PLAYER : PLAYER_ENTRY;

  type ResponseSpec = ReturnType<typeof json> | ReturnType<typeof sse> | { status: number };
  const table: Array<[string, RegExp, (req: Request) => ResponseSpec]> = [
    ['GET', /^\/setup\/status$/, () => json(200, opts.setupNeeded
      ? { ...SETUP_STATUS, configured: false, llm_configured: false }
      : SETUP_STATUS)],
    ['GET', /^\/auth\/me$/, () => json(200, USER)],
    ['POST', /^\/auth\/login$/, () => json(200, AUTH_TOKENS)],
    ['POST', /^\/auth\/register$/, () => json(200, AUTH_TOKENS)],
    ['GET', /^\/novels$/, () => json(200, NOVELS)],
    ['GET', /^\/novels\/[^/]+$/, () => json(200, NOVEL)],
    ['GET', /^\/novels\/[^/]+\/status$/, () => json(200, { status: 'ready', total_chapters: 5 })],
    ['GET', /^\/novels\/[^/]+\/chapters$/, () => json(200, [CHAPTER])],
    ['GET', /^\/novels\/[^/]+\/chapters\/[^/]+$/, () => json(200, CHAPTER)],
    ['GET', /^\/novels\/[^/]+\/characters$/, () => json(200, CHARACTERS)],
    ['GET', /^\/progress\/[^/]+$/, () => json(200, PROGRESS)],
    ['PUT', /^\/progress\/[^/]+$/, () => json(200, PROGRESS)],
    ['GET', /^\/narrative\/[^/]+\/player-entry$/, () => json(200, entry)],
    ['PUT', /^\/narrative\/[^/]+\/player-entry$/, () => json(200, PLAYER_ENTRY)],
    ['GET', /^\/narrative\/[^/]+\/chapters\/[^/]+$/, () => json(200, EFFECTIVE_CHAPTER)],
    ['GET', /^\/narrative\/[^/]+\/world-state$/, () => json(200, WORLD_STATE)],
    ['GET', /^\/narrative\/[^/]+\/world$/, () => (openWorld ? json(200, OPEN_WORLD) : { status: 404 })],
    ['GET', /^\/narrative\/[^/]+\/[^/]+$/, () => json(200, NODE)],
    ['POST', /^\/narrative\/[^/]+\/world$/, () => json(200, OPEN_WORLD)],
    ['POST', /^\/narrative\/[^/]+\/world\/turns$/, () => json(200, WORLD_TURN_RESULT)],
    ['POST', /^\/narrative\/choose$/, () => json(200, CHOICE_RESULT)],
    ['POST', /^\/chat\/[^/]+\/stream$/, (req) => {
      const turnId = (req.headers()['idempotency-key'] ?? '') as string;
      return turnId ? CHAT_DELTA_STREAM(turnId) : CHAT_STREAM;
    }],
    ['GET', /^\/settings\/llm$/, () => json(200, LLM_SETTINGS)],
    ['PUT', /^\/settings\/llm$/, () => json(200, LLM_SETTINGS)],
    ['POST', /^\/setup\/init$/, () => json(200, { ok: true })],
  ];

  await page.route('**/api/**', async (route) => {
    const request = route.request();
    const method = request.method();
    const pathname = new URL(request.url()).pathname.replace(/^\/api/, '') || '/';
    for (const [m, pattern, build] of table) {
      if (m === method && pattern.test(pathname)) {
        const response = build(request);
        if (response.status === 404) {
          await route.fulfill({ status: 404, contentType: 'application/json', body: '{}' });
          return;
        }
        await route.fulfill(response);
        return;
      }
    }
    // Deterministic fallback: anything unstubbed is a test bug — fail loudly.
    await route.fulfill({
      status: 404,
      contentType: 'application/json',
      body: JSON.stringify({ code: 'stub_missing', message: 'no stub for ' + method + ' ' + pathname }),
    });
  });

  // Deterministic typography: block Google Fonts so local runs match CI
  // (no network) — the gate scans the fallback-font rendering. Recorded.
  await page.route('https://fonts.googleapis.com/**', (route) => route.abort());
  await page.route('https://fonts.gstatic.com/**', (route) => route.abort());

  // Authenticated session: real guard passes; /auth/me returns USER.
  await page.addInitScript(() => {
    localStorage.setItem('auth_token', 'e2e-access-token');
    localStorage.setItem('refresh_token', 'e2e-refresh-token');
  });
}

export { JOURNAL_ENTRY };
