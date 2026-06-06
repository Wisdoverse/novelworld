// ─── 小说相关 ─────────────────────────────────────────────────────────────────

export type NovelStatus = 'pending' | 'parsing' | 'ready' | 'error';
export type DeviationMode = 'canon' | 'creative' | 'remix';

export interface Novel {
  id: string;
  user_id: string;
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
  aliases: string[];
  role: CharacterRole;
  description?: string;
  personality?: string;
  background?: string;
  speaking_style?: string;
  appearance?: string;
  avatar_url?: string;
  avatar_status: AvatarStatus;
  first_appearance_chapter?: number;
}

// ─── 对话 ─────────────────────────────────────────────────────────────────────

export interface ChatMessage {
  id: string;
  role: 'user' | 'character';
  content: string;
  character_id: string;
  created_at: string;
}

// ─── 记忆 ─────────────────────────────────────────────────────────────────────

export type MemoryLayer = 'short' | 'mid' | 'long' | 'permanent';

export interface Memory {
  id: string;
  layer: MemoryLayer;
  content: string;
  importance: number;
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
  choices: NarrativeChoice[];
}

// ─── 世界状态 ─────────────────────────────────────────────────────────────────

export interface WorldState {
  user_id: string;
  novel_id: string;
  state: {
    choices: Array<{
      chapter: number;
      choice: string;
      consequence: string;
      timestamp: string;
    }>;
    relationships: Record<string, { score: number; last_change: string }>;
    world_events: string[];
  };
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
  token_type: 'Bearer';
}
