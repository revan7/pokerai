import { expect, test } from 'vitest';
import { Channel } from '@tauri-apps/api/core';
import type { RecommendationEvent } from '../ipc/types.gen';
import { tauriBackend } from '../ipc/backend';
import { FakeBackend } from './fakeBackend';
import { installMockIpc } from './mockIpc';
import { identity, recommendation } from './fixtures';

test('backend_uses_channel_and_exact_command_names', async () => {
  const fake = new FakeBackend(); installMockIpc(fake);
  const events: unknown[] = [];
  const id = await tauriBackend.recommend(e => events.push(e));
  fake.emit({kind:'Final',...recommendation()});
  expect(id).toEqual(identity);
  expect(events).toEqual([{kind:'Final',...recommendation()}]);
  await tauriBackend.cancel(id.decision_id);
  expect(fake.calls.at(-1)).toEqual(['cancel', { decision_id: identity.decision_id }]);
});

// The pinned mocks module does not export `runCallback`; do not import a nonexistent
// helper or reach into private globals. Production relies on Tauri's Channel framing,
// pinned here against @tauri-apps/api-v2.11.1/packages/api/src/mocks.ts.
test('channel_uses_v2_serialization_marker', () => {
  installMockIpc(new FakeBackend());
  const channel = new Channel<RecommendationEvent>();
  expect(channel.toJSON()).toBe(`__CHANNEL__:${channel.id}`);
});
