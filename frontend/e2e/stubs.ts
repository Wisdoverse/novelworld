import type { Page, Request } from '@playwright/test';
import {
  AUTH_TOKENS, CHAPTER, CHARACTERS, CHARACTER_PROGRESS, CHOICE_RESULT, EFFECTIVE_CHAPTER,
  GAME_RULE_TEMPLATE, JOURNAL_ENTRY, LLM_SETTINGS, LLM_USAGE, NODE, NOVEL, NOVELS, OPEN_WORLD, PLAYER_ENTRY,
  PLAYER_ENTRY_NO_PLAYER, PROGRESS, SETUP_STATUS, WORLD_STATE, WORLD_TURN_RESULT, USER,
} from './fixtures';

export interface StubOptions {
  /** Seed an authenticated browser session. Defaults to true. */
  authenticated?: boolean;
  /** /setup/status reports configured:false so the app boots into SetupPage. */
  setupNeeded?: boolean;
  /** Open-world view present on the reader page (WorldDashboard + WorldActionForm render). */
  openWorld?: boolean;
  /** Player-entry has no player yet (PlayerEntryForm renders). */
  entryRequired?: boolean;
  /** Progress adopts a canonical character identity (boundary mode). */
  characterIdentity?: boolean;
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
  let setupNeeded = opts.setupNeeded ?? false;
  let entry = opts.entryRequired ? PLAYER_ENTRY_NO_PLAYER : PLAYER_ENTRY;
  const progress = opts.characterIdentity ? CHARACTER_PROGRESS : PROGRESS;
  let effectiveChapter = EFFECTIVE_CHAPTER;
  let worldState = opts.characterIdentity
    ? CHOICE_RESULT.world_state
    : opts.openWorld
      ? OPEN_WORLD.world_state
      : WORLD_STATE;
  let openWorld = opts.openWorld ? OPEN_WORLD : null;

  type ResponseSpec = ReturnType<typeof json> | ReturnType<typeof sse> | { status: number };
  const table: Array<[string, RegExp, (req: Request) => ResponseSpec]> = [
    ['GET', /^\/setup\/status$/, () => json(200, setupNeeded
      ? { ...SETUP_STATUS, configured: false, admin_configured: false, llm_configured: false }
      : SETUP_STATUS)],
    ['GET', /^\/auth\/me$/, () => json(200, USER)],
    ['POST', /^\/auth\/login$/, () => json(200, AUTH_TOKENS)],
    ['POST', /^\/auth\/register$/, () => json(200, AUTH_TOKENS)],
    ['GET', /^\/novels$/, () => json(200, NOVELS)],
    ['GET', /^\/novels\/catalog$/, () => json(200, NOVELS)],
    ['GET', /^\/novels\/[^/]+$/, () => json(200, NOVEL)],
    ['GET', /^\/novels\/[^/]+\/status$/, () => json(200, { status: 'ready', total_chapters: 5 })],
    ['GET', /^\/novels\/[^/]+\/chapters$/, () => json(200, [CHAPTER])],
    ['GET', /^\/novels\/[^/]+\/chapters\/[^/]+$/, () => json(200, CHAPTER)],
    ['GET', /^\/novels\/[^/]+\/characters$/, () => json(200, CHARACTERS)],
    ['GET', /^\/progress\/[^/]+$/, () => json(200, progress)],
    ['PUT', /^\/progress\/[^/]+$/, () => json(200, progress)],
    ['GET', /^\/narrative\/[^/]+\/player-entry$/, () => json(200, entry)],
    ['PUT', /^\/narrative\/[^/]+\/player-entry$/, () => {
      entry = PLAYER_ENTRY;
      worldState = WORLD_STATE;
      return json(200, entry);
    }],
    ['POST', /^\/narrative\/[^/]+\/game-rules$/, () => json(200, GAME_RULE_TEMPLATE)],
    ['GET', /^\/narrative\/[^/]+\/chapters\/[^/]+$/, () => json(200, effectiveChapter)],
    ['GET', /^\/narrative\/[^/]+\/world-state$/, () => json(200, worldState)],
    ['GET', /^\/narrative\/[^/]+\/world$/, () => (openWorld ? json(200, openWorld) : { status: 404 })],
    ['GET', /^\/narrative\/[^/]+\/[^/]+$/, () => json(200, NODE)],
    ['POST', /^\/narrative\/[^/]+\/world$/, () => {
      openWorld = OPEN_WORLD;
      worldState = OPEN_WORLD.world_state;
      return json(200, openWorld);
    }],
    ['POST', /^\/narrative\/[^/]+\/world\/turns$/, () => {
      worldState = WORLD_TURN_RESULT.world_state;
      openWorld = {
        ...OPEN_WORLD,
        session: { ...OPEN_WORLD.session, turn_number: 2 },
        world_state: worldState,
        journal: [
          ...OPEN_WORLD.journal,
          { ...JOURNAL_ENTRY, turn_id: WORLD_TURN_RESULT.turn_id, turn_number: 2 },
        ],
      };
      return json(200, WORLD_TURN_RESULT);
    }],
    ['POST', /^\/narrative\/choose$/, () => {
      worldState = CHOICE_RESULT.world_state;
      effectiveChapter = {
        chapter_number: CHOICE_RESULT.chapter_number,
        content: CHOICE_RESULT.chapter_content,
        generated: true,
      };
      return json(200, CHOICE_RESULT);
    }],
    ['GET', /^\/chat\/[^/]+\/history$/, () => json(200, { messages: [], count: 0 })],
    ['POST', /^\/chat\/[^/]+\/stream$/, (req) => {
      const turnId = (req.headers()['idempotency-key'] ?? '') as string;
      return turnId ? CHAT_DELTA_STREAM(turnId) : CHAT_STREAM;
    }],
    ['GET', /^\/settings\/llm$/, () => json(200, LLM_SETTINGS)],
    ['GET', /^\/settings\/llm\/usage$/, () => json(200, LLM_USAGE)],
    ['PUT', /^\/settings\/llm$/, () => json(200, LLM_SETTINGS)],
    ['POST', /^\/setup\/init$/, () => {
      setupNeeded = false;
      return json(200, AUTH_TOKENS);
    }],
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

  if (opts.authenticated !== false) {
    // Authenticated session: real guard passes; /auth/me returns USER.
    await page.addInitScript(() => {
      localStorage.setItem('auth_token', 'e2e-access-token');
      localStorage.setItem('refresh_token', 'e2e-refresh-token');
    });
  }
}

export { JOURNAL_ENTRY };
