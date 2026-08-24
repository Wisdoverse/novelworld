// E2E fixture data for the browser accessibility gate (issue #167).
// Shapes mirror frontend/src/shared/types/index.ts and the entity api DTOs.

export const USER = {
  id: 'user-1',
  email: 'reader@example.com',
  name: '测试读者',
  role: 'admin',
};

export const NOVEL = {
  id: 'novel-1',
  user_id: 'user-1',
  title: '星海拾遗',
  author: '晨星',
  total_chapters: 5,
  status: 'ready',
  deviation_mode: 'canon',
  description: '一部关于星辰与旅人的长篇小说，用于可访问性扫描。',
  world_summary: '世界设定摘要',
  genre: '幻想',
  created_at: '2025-01-01T00:00:00Z',
  updated_at: '2025-01-01T00:00:00Z',
};

export const NOVELS = [NOVEL];

export const CHAPTER_TEXT = Array.from(
  { length: 40 },
  (_, i) => '第' + (i + 1) + '段：夜色笼罩北塔，旅人沿石阶而上，海风从港口的桅杆间穿过，带来盐与旧帆的气息。',
).join('\n');

export const CHAPTER = {
  id: 'ch-1',
  novel_id: 'novel-1',
  chapter_number: 1,
  title: '第一章 北塔来信',
  content: CHAPTER_TEXT,
  summary: '旅人抵达北塔，收到一封来自海港的信。',
  is_key_node: true,
  key_node_description: '旅人决定是否回应信中的召唤。',
};

export const CHARACTERS = [
  {
    id: 'char-1',
    novel_id: 'novel-1',
    name: '林晚',
    aliases: ['晚晚'],
    role: 'protagonist',
    description: '北塔的守灯人，沉默而可靠。',
    personality: '谨慎、寡言',
    background: '自幼守塔，熟悉每一阵风。',
    speaking_style: '简短而坚定',
    appearance: '灰袍，腰间挂着铜灯。',
    avatar_status: 'ready',
    first_appearance_chapter: 1,
  },
  {
    id: 'char-2',
    novel_id: 'novel-1',
    name: '老船长',
    aliases: [],
    role: 'supporting',
    description: '往来海港与北塔的船长。',
    avatar_status: 'pending',
    first_appearance_chapter: 1,
  },
];

export const PROGRESS = {
  id: 'p-1',
  user_id: 'user-1',
  novel_id: 'novel-1',
  current_chapter: 1,
  reader_identity_type: 'self',
  deviation_mode: 'canon',
  last_read_at: '2025-01-01T00:00:00Z',
};

// H4 identity boundary: character-identity readers keep conversation and may
// read an exact committed branch; they must never create a new branch or see
// open-world/player-entry agency.
export const CHARACTER_PROGRESS = {
  ...PROGRESS,
  id: 'p-char-1',
  reader_identity_type: 'character',
  reader_identity: '林晚',
  reader_character_id: 'char-1',
};

export const EFFECTIVE_CHAPTER = {
  chapter_number: 1,
  content: CHAPTER_TEXT,
  generated: false,
};

export const NODE = {
  id: 'node-1',
  novel_id: 'novel-1',
  chapter_number: 1,
  description: '信纸上的墨迹尚未干透：\u201c若你愿意，明日黎明随船出海。\u201d',
  anchor_quote: '\u201c海在等一个答案。\u201d',
  choices: [
    { index: 0, text: '收下信，答应出海', hint: '开启海港线' },
    { index: 1, text: '把信放回原处', hint: '留在北塔' },
  ],
};

export const PLAYER = {
  id: 'pe-1',
  user_id: 'user-1',
  novel_id: 'novel-1',
  canonical_checkpoint_chapter: 1,
  name: '无名旅人',
  background: '一个不记得来路的旅人。',
  capabilities: ['阅读星图'],
  location_id: 'loc-1',
  inventory: [],
  relationships: {},
  faction_standing: {},
  discovered_knowledge: [],
  created_at: '2025-01-01T00:00:00Z',
};

export const PLAYER_ENTRY = {
  player: PLAYER,
  checkpoint_chapter: 1,
  locations: [
    { id: 'loc-1', name: '北塔' },
    { id: 'loc-2', name: '海港' },
  ],
};

export const PLAYER_ENTRY_NO_PLAYER = {
  player: null,
  checkpoint_chapter: 1,
  locations: PLAYER_ENTRY.locations,
};

export const GAME_RULE_TEMPLATE = {
  novel_id: 'novel-1',
  canon_model_version: 1,
  schema_version: 1,
  prompt_version: 'novel-game-rules-v1',
  minimum_score: 8,
  maximum_score: 15,
  point_budget: 30,
  attributes: [
    { key: 'qinggong', label: '轻功', description: '在屋脊与山道间腾挪。', default_score: 10, source_chapters: [1] },
    { key: 'dongcha', label: '洞察', description: '辨认江湖话术与隐藏线索。', default_score: 10, source_chapters: [1] },
    { key: 'renmai', label: '人脉', description: '借助门派声望与江湖关系。', default_score: 10, source_chapters: [1] },
  ],
  action_rules: [
    ['travel', 'qinggong'], ['investigate', 'dongcha'], ['converse', 'renmai'],
    ['ally', 'renmai'], ['oppose', 'dongcha'], ['advance_thread', 'dongcha'],
    ['resolve_thread', 'qinggong'], ['pursue_goal', 'qinggong'],
  ].map(([kind, attribute_key]) => ({
    kind, attribute_key, difficulty_class: 12,
    description: '依据小说世界规则完成不确定行动。', source_chapters: [1],
  })),
};

export const WORLD_STATE = {
  user_id: 'user-1',
  novel_id: 'novel-1',
  updated_at: '2025-01-01T00:00:00Z',
  state: {
    choices: [],
    relationships: {},
    player_entity: PLAYER,
    world_events: [
      {
        id: 'we-1',
        origin: 'player',
        turn_id: 't-1',
        turn_number: 1,
        world_time: 10,
        summary: '旅人离开北塔，沿海岸线走向海港。',
        actor_character_ids: [],
        location_id: 'loc-2',
      },
    ],
    locations: { 'loc-1': { state: '灯火通明', reason: '照常' } },
    threads: {
      'th-1': { status: 'open', description: '信中的召唤', origin: 'canon' },
    },
    open_world: null,
  },
};

export const SESSION = {
  schema_version: 1,
  entry_context: {
    model_version: 1,
    checkpoint_chapter: 1,
    unlocked_through_chapter: 1,
    characters: [
      { id: 'char-1', name: '林晚' },
      { id: 'char-2', name: '老船长' },
    ],
    locations: [
      { id: 'loc-1', name: '北塔' },
      { id: 'loc-2', name: '海港' },
    ],
    factions: [],
    hard_rules: [{ id: 'hr-1', description: '死者无法回应' }],
    dead_character_ids: [],
    threads: [{ id: 'th-1', name: '信中的召唤' }],
    scheduled_events: [],
    character_goals: [],
  },
  world_time: 10,
  turn_number: 1,
  canonical_events: [
    {
      id: 'ce-1',
      sequence: 1,
      summary: '商船在三日后抵达海港。',
      character_ids: ['char-2'],
      location_ids: ['loc-2'],
      faction_ids: [],
      death_character_ids: [],
      source_chapters: [2],
      status: 'witnessed',
      reason: null,
    },
  ],
  dead_character_ids: [],
  character_perceptions: {},
};

export const JOURNAL_ENTRY = {
  turn_id: 't-1',
  turn_number: 1,
  memory_projection_status: 'saved' as const,
  action: { kind: 'travel', target_id: 'loc-2', intent: '前往海港' },
  transition: {
    schema_version: 1,
    prompt_version: '1.0',
    canon_model_version: 1,
    canonical_checkpoint_chapter: 1,
    rendered_narrative: '旅人沿山路下行，港口的灯火渐渐清晰。',
    events: [{ summary: '旅人抵达海港', actor_character_ids: [], location_id: 'loc-2' }],
    relationship_changes: [],
    location_changes: [],
    thread_changes: [],
    player_location_id: 'loc-2',
    inventory_additions: [],
    inventory_removals: [],
    knowledge_discoveries: [],
    faction_changes: [],
    canonical_event_change: null,
  },
  created_at: '2025-01-01T00:00:00Z',
  completed_at: '2025-01-01T00:00:00Z',
};

const LONG_TIMELINE_TOKEN = 'A'.repeat(500);
const OPEN_WORLD_STATE = {
  ...WORLD_STATE,
  state: {
    ...WORLD_STATE.state,
    choices: [{
      chapter: 1,
      choice: `长选择起点\n${LONG_TIMELINE_TOKEN}`,
      consequence: `长选择投影起点\n${LONG_TIMELINE_TOKEN}`,
    }],
  },
};
const OPEN_WORLD_JOURNAL_ENTRY = {
  ...JOURNAL_ENTRY,
  action: {
    ...JOURNAL_ENTRY.action,
    intent: `前往海港\n${LONG_TIMELINE_TOKEN}`,
  },
  transition: {
    ...JOURNAL_ENTRY.transition,
    rendered_narrative: `${JOURNAL_ENTRY.transition.rendered_narrative}\n长行动投影起点\n${LONG_TIMELINE_TOKEN}`,
  },
};

export const OPEN_WORLD = {
  player: PLAYER,
  session: SESSION,
  world_state: OPEN_WORLD_STATE,
  journal: [OPEN_WORLD_JOURNAL_ENTRY],
};

export const WORLD_TURN_RESULT = {
  turn_id: 't-2',
  action: { kind: 'travel', target_id: 'loc-2', intent: '前往海港' },
  transition: JOURNAL_ENTRY.transition,
  world_state: WORLD_STATE,
  memory_projection_status: 'saved' as const,
};

export const CHOICE_RESULT = {
  chapter_number: 1,
  consequence: '旅人收下信，约定黎明出海。',
  transition: JOURNAL_ENTRY.transition,
  chapter_content: CHAPTER_TEXT,
  world_state: {
    ...WORLD_STATE,
    state: {
      ...WORLD_STATE.state,
      choices: [{
        node_id: NODE.id,
        chapter: NODE.chapter_number,
        choice_index: NODE.choices[0].index,
        choice: NODE.choices[0].text,
        consequence: '旅人收下信，约定黎明出海。',
        canon_model_version: 1,
        canonical_checkpoint_chapter: 1,
        timestamp: '2025-01-01T00:00:00Z',
      }],
    },
  },
};

export const LLM_SETTINGS = {
  scope: 'platform',
  provider: 'deepseek',
  model: 'deepseek-v4-flash',
  thinking_enabled: false,
  api_key_configured: true,
};

export const LLM_USAGE = {
  contract: 1,
  scope: 'platform',
  window_days: 30,
  tokens: {
    input: '3000',
    cached_input: '1000',
    uncached_input: '2000',
    output: '500',
    total: '3500',
  },
  costs: {
    usd_micros: '450000',
    cny_micros: '3240000',
  },
  unpriced_tokens: '0',
};

export const AUTH_TOKENS = {
  user: USER,
  access_token: 'access-token',
  refresh_token: 'refresh-token',
  token_type: 'Bearer',
};

export const SETUP_STATUS = {
  contract: 3,
  configured: true,
  llm_configured: true,
};
