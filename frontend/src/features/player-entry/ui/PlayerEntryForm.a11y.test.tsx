import React from 'react';
import { render } from '@testing-library/react';
import { describe, it, vi } from 'vitest';
import { expectNoA11yViolations } from '@/a11y';
import { PlayerEntryForm } from './PlayerEntryForm';

describe('PlayerEntryForm a11y', () => {
  it('has no axe violations on the entry form', async () => {
    const { container } = render(
      <PlayerEntryForm
        checkpointChapter={2}
        unlockedThroughChapter={2}
        locations={[{ id: 'tower', name: '北塔' }]}
        isPending={false}
        error={undefined}
        onCheckpointChange={vi.fn()}
        onSubmit={vi.fn()}
      />,
    );
    await expectNoA11yViolations(container);
  });
});