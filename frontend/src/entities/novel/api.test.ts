import { describe, expect, it } from 'vitest';
import type { Character } from '@/shared/types';
import {
  buildNovelUploadFormData,
  isCharacterAvailable,
  shouldPollNovelList,
  validateNovelFile,
} from './api';

const character = (firstAppearance?: number): Character => ({
  id: crypto.randomUUID(),
  novel_id: 'novel',
  name: 'Character',
  role: 'supporting',
  aliases: [],
  avatar_status: 'pending',
  first_appearance_chapter: firstAppearance,
});

describe('novel file uploads', () => {
  it('accepts TXT and EPUB within their size limits', () => {
    expect(validateNovelFile(new File(['text'], 'story.txt', { type: 'text/plain' }))).toBeNull();
    expect(validateNovelFile(new File(['epub'], 'story.EPUB', { type: 'application/epub+zip' }))).toBeNull();
    expect(validateNovelFile(new File(['x'], 'story.docx'))).toContain('TXT、EPUB');
  });

  it('builds the multipart upload contract without a client identity', () => {
    const file = new File(['story'], 'story.epub', { type: 'application/epub+zip' });
    const form = buildNovelUploadFormData({
      title: 'Story',
      author: 'Author',
      deviationMode: 'canon',
      file,
    });
    expect(form.get('title')).toBe('Story');
    expect(form.get('author')).toBe('Author');
    expect(form.get('deviation_mode')).toBe('canon');
    expect(form.get('file')).toBe(file);
    expect(form.has('user_id')).toBe(false);
  });
});

describe('novel ingestion status', () => {
  it('polls while background ingestion is pending or parsing', () => {
    const novel = (status: 'pending' | 'parsing' | 'ready' | 'error') => ({
      id: 'novel',
      user_id: 'user',
      title: 'Story',
      total_chapters: 0,
      status,
      deviation_mode: 'canon' as const,
      created_at: new Date(0).toISOString(),
      updated_at: new Date(0).toISOString(),
    });
    expect(shouldPollNovelList([novel('parsing')])).toBe(true);
    expect(shouldPollNovelList([novel('pending')])).toBe(true);
    expect(shouldPollNovelList([novel('ready')])).toBe(false);
    expect(shouldPollNovelList([novel('error')])).toBe(false);
  });
});

describe('character visibility', () => {
  it('fails closed and follows forward and rewind progress', () => {
    expect(isCharacterAvailable(character(undefined), 5)).toBe(false);
    expect(isCharacterAvailable(character(0), 5)).toBe(false);
    expect(isCharacterAvailable(character(5), 4)).toBe(false);
    expect(isCharacterAvailable(character(5), 5)).toBe(true);
    expect(isCharacterAvailable(character(5), 1)).toBe(false);
  });
});
