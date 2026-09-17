import { render, screen } from '@testing-library/react';
import { expect, test } from 'vitest';
import { App } from '../app';
test('desktop_shell_renders', () => {
  render(<App />);
  expect(screen.getByRole('heading', { name: 'PokerAI' })).toBeVisible();
});
