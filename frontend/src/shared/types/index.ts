// ─── 小说相关 ─────────────────────────────────────────────────────────────────

export type NovelStatus = 'pending' | 'parsing' | 'ready' | 'error';
export type DeviationMode = 'canon' | 'creative' | 'remix';

export interface Novel {
  id: string;
  /** Legacy clients may receive uploader attribution; access never depends on it. */
  user_id?: string;
  title: string;
  author?: string;
  cover_url?: string;
  description?: string;
  world_summary?: string;
  genre?: string;
  total_chapters: number;
  status: NovelStatus;
  parse_error?: string;
  deviation_mode: DeviationMode;
  created_at: string;
  updated_at: string;
}

// ─── 章节 ─────────────────────────────────────────────────────────────────────

export interface Chapter {
  id: string;
  novel_id: string;
  chapter_number: number;
  title?: string;
  content: string;
  summary?: string;
  is_key_node: boolean;
  key_node_description?: string;
}

// ─── 角色 ─────────────────────────────────────────────────────────────────────

export type CharacterRole = 'protagonist' | 'antagonist' | 'supporting' | 'minor';
export type AvatarStatus = 'pending' | 'generating' | 'ready' | 'error';

export interface Character {
  id: string;
  novel_id: string;
  name: string;
  aliases?: string[];
  role?: CharacterRole;
  description?: string;
  personality?: string;
  background?: string;
  speaking_style?: string;
  appearance?: string;
  avatar_url?: string;
  avatar_status?: AvatarStatus;
  first_appearance_chapter?: number;
  persona_source_chapter_high_water?: number;
}

// ─── 对话 ─────────────────────────────────────────────────────────────────────

export interface ChatMessage {
  id: string;
  turn_id?: string | null;
  role: 'user' | 'character';
  content: string;
  character_id: string;
  chapter_context?: number | null;
  created_at: string;
}

// ─── 叙事分支 ─────────────────────────────────────────────────────────────────

export interface NarrativeChoice {
  index: number;
  text: string;
  hint: string;
  generated_consequence?: string;
}

export interface NarrativeNode {
  id: string;
  novel_id: string;
  chapter_number: number;
  description: string;
  anchor_quote?: string;
  choices: NarrativeChoice[];
}

// ─── 世界状态 ─────────────────────────────────────────────────────────────────

export interface PlayerEntity {
  id: string;
  user_id: string;
  novel_id: string;
  canonical_checkpoint_chapter: number;
  name: string;
  background: string;
  capabilities: string[];
  location_id: string;
  inventory: string[];
  relationships: Record<string, { score: number; last_change: string }>;
  faction_standing: Record<string, number>;
  discovered_knowledge: string[];
  rules?: PlayerRuleProfile;
  created_at: string;
}

export type ResolutionMode = 'narrative' | 'advanced';

export interface PlayerRuleProfile {
  mode: ResolutionMode;
  canon_model_version: number | null;
  template_schema_version: number | null;
  template_prompt_version: string | null;
  attributes: Record<string, number>;
}

export interface GameAttribute {
  key: string;
  label: string;
  description: string;
  default_score: number;
  source_chapters: number[];
}

export interface GameActionRule {
  kind: WorldActionKind;
  attribute_key: string;
  difficulty_class: number;
  description: string;
  source_chapters: number[];
}

export interface GameRuleTemplate {
  novel_id: string;
  canon_model_version: number;
  schema_version: number;
  prompt_version: string;
  minimum_score: number;
  maximum_score: number;
  point_budget: number;
  attributes: GameAttribute[];
  action_rules: GameActionRule[];
}

export interface PlayerEntry {
  player: PlayerEntity | null;
  checkpoint_chapter: number;
  locations: Array<{ id: string; name: string }>;
  game_rules?: GameRuleTemplate | null;
}

export type WorldActionKind =
  | 'travel'
  | 'investigate'
  | 'converse'
  | 'ally'
  | 'oppose'
  | 'advance_thread'
  | 'resolve_thread'
  | 'pursue_goal';

export interface WorldAction {
  kind: WorldActionKind;
  target_id: string | null;
  intent: string;
}

export interface WorldEntryContext {
  model_version: number;
  checkpoint_chapter: number;
  unlocked_through_chapter: number;
  characters: Array<{ id: string; name: string }>;
  locations: Array<{ id: string; name: string }>;
  factions: Array<{ id: string; name: string }>;
  hard_rules: Array<{ id: string; description: string }>;
  dead_character_ids: string[];
  threads: Array<{ id: string; name: string }>;
  scheduled_events: ScheduledCanonEvent[];
  character_goals: Array<{
    id: string;
    character_id: string;
    description: string;
    source_chapters: number[];
  }>;
}

export interface ScheduledCanonEvent {
  id: string;
  sequence: number;
  summary: string;
  character_ids: string[];
  location_ids: string[];
  faction_ids: string[];
  death_character_ids: string[];
  source_chapters: number[];
}

export type CanonicalEventStatus =
  | 'scheduled'
  | 'occurred'
  | 'witnessed'
  | 'assisted'
  | 'obstructed'
  | 'delayed'
  | 'redirected'
  | 'prevented';

export interface CanonicalEventState extends ScheduledCanonEvent {
  status: CanonicalEventStatus;
  reason: string | null;
}

export interface WorldSession {
  schema_version: number;
  entry_context: WorldEntryContext;
  world_time: number;
  turn_number: number;
  canonical_events: CanonicalEventState[];
  dead_character_ids: string[];
  character_perceptions: Record<string, string>;
  game_rules?: GameRuleTemplate | null;
}

export interface ActionCheck {
  schema_version: number;
  canon_model_version: number;
  template_prompt_version: string;
  attribute_key: string;
  attribute_label: string;
  score: number;
  modifier: number;
  roll: number;
  difficulty_class: number;
  total: number;
  succeeded: boolean;
}

export interface WorldTurnTransition {
  schema_version: number;
  prompt_version: string;
  canon_model_version: number;
  canonical_checkpoint_chapter: number;
  rendered_narrative: string;
  events: Array<{
    summary: string;
    actor_character_ids: string[];
    location_id: string | null;
  }>;
  relationship_changes: Array<{ character_id: string; delta: number; reason: string }>;
  location_changes: Array<{ location_id: string; state: string; reason: string }>;
  thread_changes: Array<{
    thread_id: string;
    status: 'open' | 'resolved';
    description: string;
  }>;
  player_location_id: string | null;
  inventory_additions: string[];
  inventory_removals: string[];
  knowledge_discoveries: string[];
  faction_changes: Array<{ faction_id: string; delta: number; reason: string }>;
  canonical_event_change: {
    event_id: string;
    status: CanonicalEventStatus;
    reason: string;
  } | null;
}

export interface WorldTurnJournalEntry {
  turn_id: string;
  turn_number: number;
  memory_projection_status: 'pending' | 'saved' | 'skipped';
  action: WorldAction;
  resolution?: ActionCheck | null;
  transition: WorldTurnTransition;
  created_at: string;
  completed_at: string;
}

export interface WorldState {
  user_id: string;
  novel_id: string;
  updated_at: string;
  state: {
    choices: Array<{
      node_id?: string;
      chapter: number;
      choice_index?: number;
      choice: string;
      consequence: string;
      canon_model_version?: number;
      canonical_checkpoint_chapter?: number;
      timestamp?: string;
    }>;
    relationships?: Record<string, { score: number; last_change: string }>;
    player_entity?: PlayerEntity;
    world_events: Array<string | {
      id: string;
      chapter?: number;
      origin?: 'player';
      turn_id?: string;
      turn_number?: number;
      world_time?: number;
      summary: string;
      actor_character_ids: string[];
      location_id: string | null;
    }>;
    locations?: Record<string, { state: string; reason: string }>;
    threads?: Record<string, {
      status: 'open' | 'resolved';
      description: string;
      origin?: 'canon' | 'player';
      turn_id?: string;
    }>;
    open_world?: WorldSession;
    reader_reputation?: Record<string, unknown>;
  };
}

export interface OpenWorldView {
  player: PlayerEntity;
  session: WorldSession;
  world_state: WorldState;
  journal: WorldTurnJournalEntry[];
}

export interface WorldTurnResult {
  turn_id: string;
  /** Present on the terminal H3 contract; absent on an older Narrative service. */
  memory_projection_status?: 'saved' | 'skipped';
  action: WorldAction;
  resolution?: ActionCheck | null;
  transition: WorldTurnTransition;
  world_state: WorldState;
}

// ─── 阅读进度 ─────────────────────────────────────────────────────────────────

export type IdentityType = 'self' | 'character';

export interface ReadingProgress {
  id: string;
  user_id: string;
  novel_id: string;
  current_chapter: number;
  reader_identity?: string;
  reader_identity_type: IdentityType;
  reader_character_id?: string;
  deviation_mode: DeviationMode;
  last_read_at: string;
}

// ─── 用户 ─────────────────────────────────────────────────────────────────────

export interface User {
  id: string;
  email: string;
  name?: string;
  avatar_url?: string;
  role: 'user' | 'admin';
}

export interface AuthTokens {
  access_token: string;
  refresh_token: string;
  token_type: 'Bearer';
}
