import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { BranchChoice } from './BranchChoice';

const node = {
  id: 'node-1',
  novel_id: 'novel-1',
  chapter_number: 1,
  description: '三人即将在桃园立下盟誓。',
  choices: [
    { index: 0, text: '立即结义', hint: '誓言将牵动天下……' },
    { index: 1, text: '暂缓盟誓', hint: '迟疑也会改变命运……' },
  ],
};

describe('BranchChoice', () => {
  it('submits a selected choice instead of silently dismissing the node', async () => {
    const onChoose = vi.fn().mockResolvedValue(undefined);
    render(<BranchChoice node={node} onChoose={onChoose} />);

    fireEvent.click(screen.getByRole('button', { name: /立即结义/ }));

    await waitFor(() => expect(onChoose).toHaveBeenCalledWith(node.choices[0]));
  });

  it('keeps the committed choice and its consequence visible', () => {
    render(
      <BranchChoice
        node={node}
        onChoose={vi.fn().mockResolvedValue(undefined)}
        selectedChoiceIndex={0}
        consequence="三人的誓言从此改变了天下大势。"
      />,
    );

    expect(screen.getByText('你的行动改变了后续故事')).toBeTruthy();
    expect(screen.getByText('三人的誓言从此改变了天下大势。')).toBeTruthy();
    expect(screen.getByRole('button', { name: /立即结义.*已选择/ }).getAttribute('aria-pressed'))
      .toBe('true');
  });

  it('locks every option while a conflicting committed result is being recovered', () => {
    const onChoose = vi.fn().mockResolvedValue(undefined);
    const onRetryRecovery = vi.fn().mockResolvedValue(undefined);
    render(
      <BranchChoice
        node={node}
        onChoose={onChoose}
        error="另一窗口已提交"
        isRecoveryLocked
        onRetryRecovery={onRetryRecovery}
      />,
    );

    expect(screen.getByRole('button', { name: /立即结义/ }).hasAttribute('disabled')).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: '重新加载已提交结果' }));
    expect(onRetryRecovery).toHaveBeenCalledOnce();
    expect(onChoose).not.toHaveBeenCalled();
  });
});
