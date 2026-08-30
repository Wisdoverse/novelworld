import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { Character } from '@/shared/types';
import { CharacterCard } from './CharacterCard';

const identity: Character = {
  id: 'character',
  novel_id: 'novel',
  name: 'Character',
  first_appearance_chapter: 1,
};

describe('CharacterCard persona boundary', () => {
  it('renders a partial character without inventing or exposing persona fields', () => {
    const { container } = render(<CharacterCard character={identity} onTalk={vi.fn()} />);

    expect(screen.getByText('Character')).toBeTruthy();
    expect(screen.getByText('暂无描述')).toBeTruthy();
    expect(screen.queryByText(/别名/)).toBeNull();
    expect(screen.queryByText(/主角|反派|配角|路人/)).toBeNull();
    expect(container.querySelector('img')).toBeNull();
  });

  it('restores the complete persona when the bounded response includes it', () => {
    const full: Character = {
      ...identity,
      aliases: ['Alias'],
      role: 'protagonist',
      description: 'Complete description',
      avatar_url: 'https://example.invalid/avatar.png',
      avatar_status: 'ready',
      persona_source_chapter_high_water: 2,
    };
    const { container } = render(<CharacterCard character={full} onTalk={vi.fn()} />);

    expect(screen.getByText('别名：Alias')).toBeTruthy();
    expect(screen.getByText('主角')).toBeTruthy();
    expect(screen.getByText('Complete description')).toBeTruthy();
    expect(container.querySelector('img')?.getAttribute('src')).toBe(full.avatar_url);
  });
});
