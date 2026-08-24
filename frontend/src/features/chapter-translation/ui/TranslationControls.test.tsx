import React from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { TranslationControls } from './TranslationControls';

describe('TranslationControls', () => {
  it('reports a pending request without claiming translated text is visible', () => {
    render(
      <TranslationControls
        active={false}
        isLoading
        isError={false}
        onToggle={vi.fn()}
        onRetry={vi.fn()}
      />,
    );

    const button = screen.getByRole('button', { name: '翻译中…' }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(button.getAttribute('aria-pressed')).toBe('false');
    expect(screen.getByRole('status').textContent).toContain('正在翻译');
  });

  it('keeps the source state and exposes retry actions after failure', () => {
    const onToggle = vi.fn();
    const onRetry = vi.fn();
    render(
      <TranslationControls
        active={false}
        isLoading={false}
        isError
        onToggle={onToggle}
        onRetry={onRetry}
      />,
    );

    const translateAgain = screen.getByRole('button', { name: '重新翻译' });
    expect(translateAgain.getAttribute('aria-pressed')).toBe('false');
    expect(screen.getByRole('alert').textContent).toContain('当前显示原文');
    fireEvent.click(translateAgain);
    fireEvent.click(screen.getByRole('button', { name: '重试' }));
    expect(onToggle).toHaveBeenCalledOnce();
    expect(onRetry).toHaveBeenCalledOnce();
  });
});
