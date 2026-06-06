import { describe, it, expect } from 'vitest';
import type { Novel, Character, ChatMessage, NarrativeNode, WorldState } from './index';

describe('Type contracts', () => {
  it('Novel has required fields', () => {
    const novel: Novel = {
      id: 'test-id',
      user_id: 'user-1',
      title: 'Test Novel',
      total_chapters: 10,
      status: 'ready',
      deviation_mode: 'canon',
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    };
    expect(novel.status).toBe('ready');
    expect(novel.total_chapters).toBeGreaterThan(0);
  });

  it('Character role is valid enum', () => {
    const roles: Array<Character['role']> = ['protagonist', 'antagonist', 'supporting', 'minor'];
    expect(roles).toHaveLength(4);
  });

  it('ChatMessage role is user or character', () => {
    const msg: ChatMessage = {
      id: '1',
      role: 'user',
      content: 'Hello',
      character_id: 'char-1',
      created_at: new Date().toISOString(),
    };
    expect(['user', 'character']).toContain(msg.role);
  });

  it('WorldState has choices array', () => {
    const ws: WorldState = {
      user_id: 'u1',
      novel_id: 'n1',
      state: {
        choices: [],
        relationships: {},
        world_events: [],
      },
    };
    expect(Array.isArray(ws.state.choices)).toBe(true);
  });
});
