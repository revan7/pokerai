import { expect, test } from 'vitest';
import { Channel, invoke } from '@tauri-apps/api/core';
import type { RecommendationEvent, BeginHand } from '../ipc/types.gen';
import type { Backend } from '../ipc/backend';
import { tauriBackend } from '../ipc/backend';
import { FakeBackend } from './fakeBackend';
import { installMockIpc } from './mockIpc';
import { identity, recommendation, config, hand } from './fixtures';

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

// R3 (review round 1): emitting to a target with no registered sink must throw a
// descriptive error, never silently no-op, so a future stale-event test cannot
// mistake "the event never reached the code under test" for "the UI discarded it."
test('emit_before_any_recommendation_throws', () => {
  const fake = new FakeBackend();
  expect(() => fake.emit({kind:'NoDecision', identity, reason:'r'})).toThrow(/no recommend sink/i);
});

test('emit_with_out_of_range_request_index_throws', async () => {
  const fake = new FakeBackend();
  const events: unknown[] = [];
  await fake.recommend(e => events.push(e));
  expect(() => fake.emit({kind:'NoDecision', identity, reason:'r'}, 5)).toThrow(/no recommend sink/i);
  expect(events).toEqual([]);
});

test('emit_still_delivers_to_a_previously_registered_sink', async () => {
  const fake = new FakeBackend();
  const events: unknown[] = [];
  await fake.recommend(e => events.push(e));
  const scripted: RecommendationEvent = {kind:'NoDecision', identity, reason:'r'};
  fake.emit(scripted, 0);
  expect(events).toEqual([scripted]);
});

// R1 (review round 1): the mock transport must validate required argument keys and
// wire shapes before dispatching to the fake, so a UI transport bug (renamed key,
// wrong scalar/collection type, malformed card, unknown action/tag discriminant)
// fails loudly here instead of passing while Rust would reject it
// (src-tauri/src/tests.rs:132 rejects invalid card text before engine dispatch).
const beginHandFixture: BeginHand = { button: 0, hero: 0, dealt: [0,1,2,3,4,5],
  stacks: [1000,1000,1000,1000,1000,1000], hero_cards: null };

const malformedArgumentCases: Array<[string, Record<string, unknown>, string]> = [
  ['set_game_config', {}, 'missing config'],
  ['set_game_config', { gameConfig: config }, 'renamed (camelCase) key'],
  ['set_game_config', { config: 'not-an-object' }, 'wrong type for config'],
  ['begin_hand', {}, 'missing begin'],
  ['begin_hand', { beginHand: beginHandFixture }, 'renamed key'],
  ['begin_hand', { begin: 'not-an-object' }, 'wrong type for begin'],
  ['begin_hand', { begin: { ...beginHandFixture, dealt: 'nope' } }, 'wrong collection type for dealt'],
  ['set_hero_cards', {}, 'missing cards'],
  ['set_hero_cards', { card: ['As','Kd'] }, 'renamed key'],
  ['set_hero_cards', { cards: ['As'] }, 'wrong shape (single card, not a pair)'],
  ['set_hero_cards', { cards: ['Zz','Kd'] }, 'invalid card text'],
  ['apply_action', {}, 'missing action'],
  ['apply_action', { action: 'fold' }, 'wrong type (string, not an object)'],
  ['apply_action', { action: { kind: 'bogus' } }, 'invalid action discriminant'],
  ['apply_action', { action: { kind: 'bet' } }, 'missing required "to" field for bet'],
  ['set_board', {}, 'missing board'],
  ['set_board', { cards: ['Kh','7d','2c'] }, 'renamed key'],
  ['set_board', { board: 'Kh7d2c' }, 'wrong collection type (string, not an array)'],
  ['set_board', { board: ['Kh','7d','2z'] }, 'invalid card in array'],
  ['cancel', {}, 'missing decision_id'],
  ['cancel', { decisionId: 90 }, 'renamed (camelCase) key'],
  ['cancel', { decision_id: '90' }, 'wrong type (string, not a number)'],
  ['set_seat_tag', {}, 'missing seat and tag'],
  ['set_seat_tag', { seat: 0, tag: 'bogus' }, 'invalid tag discriminant'],
  ['set_seat_tag', { seat: '0', tag: 'nit' }, 'wrong type for seat'],
];

test('mock_ipc_rejects_malformed_arguments_for_every_argument_bearing_command', async () => {
  for (const [command, payload, why] of malformedArgumentCases) {
    const fake = new FakeBackend();
    installMockIpc(fake);
    fake.hands.push(hand());
    await expect(invoke(command, payload), `${command}: ${why}`).rejects.toBeTruthy();
    expect(fake.calls, `${command}: ${why}`).toEqual([]);
    expect(fake.hands, `${command}: ${why} (snapshot must not be consumed)`).toHaveLength(1);
  }
});

test('mock_ipc_rejects_recommend_without_a_valid_channel', async () => {
  const fake = new FakeBackend();
  installMockIpc(fake);
  await expect(invoke('recommend', {})).rejects.toBeTruthy();
  await expect(invoke('recommend', { on_event: 'not-a-channel' })).rejects.toBeTruthy();
  expect(fake.calls).toEqual([]);
});

// R2 (review round 1): type the recorded command names from the `Backend` surface so
// a future interface change that adds/removes a wrapper fails `tsc --noEmit` here
// unless this inventory is updated in lockstep, and drive all fourteen wrappers
// through the real mockIPC/FakeBackend transport, asserting the exact emitted
// command name, arguments, response/rejection, and (for `recommend`) the Channel
// wiring. Kept in sync with the Rust registration list in
// apps/pokerai-ui/src-tauri/src/lib.rs's `generate_handler!` block, which
// apps/pokerai-ui/src-tauri/src/tests.rs's
// `registered_commands_match_the_committed_command_inventory` test reads directly
// from source and compares against this identical list of strings — a rename,
// addition, or removal on either side fails exactly one side's suite.
type BackendCommand = keyof Backend;
const COMMAND_NAMES = [
  'set_game_config', 'begin_hand', 'set_hero_cards', 'apply_action', 'set_board', 'undo',
  'recommend', 'cancel', 'finish_hand', 'abandon_hand', 'set_seat_tag',
  'presolver_status', 'presolver_pause', 'presolver_resume',
] as const satisfies readonly BackendCommand[];
// Compile-time exhaustiveness: fails `tsc --noEmit` if `Backend` gains a member not
// listed in `COMMAND_NAMES` above (an omission here would make `MissingFromInventory`
// a non-`never` string-literal type, which cannot be assigned `true`).
type MissingFromInventory = Exclude<BackendCommand, typeof COMMAND_NAMES[number]>;
const backendSurfaceFullyCovered: MissingFromInventory extends never ? true : never = true;
void backendSurfaceFullyCovered;

test('exhaustive_fourteen_command_contract', async () => {
  expect(COMMAND_NAMES).toHaveLength(14);
  const h = hand();

  { const fake = new FakeBackend(); installMockIpc(fake);
    const result = await tauriBackend.set_game_config(config);
    expect(result).toEqual({ ...config, config_revision: config.config_revision + 1 });
    expect(fake.calls).toEqual([['set_game_config', { config }]]); }

  { const fake = new FakeBackend(); installMockIpc(fake); fake.hands.push(h);
    const result = await tauriBackend.begin_hand(beginHandFixture);
    expect(result).toEqual(h);
    expect(fake.calls).toEqual([['begin_hand', { begin: beginHandFixture }]]); }

  { const fake = new FakeBackend(); installMockIpc(fake); fake.hands.push(h);
    const result = await tauriBackend.set_hero_cards(['As','Kd']);
    expect(result).toEqual(h);
    expect(fake.calls).toEqual([['set_hero_cards', { cards: ['As','Kd'] }]]); }

  { const fake = new FakeBackend(); installMockIpc(fake); fake.hands.push(h);
    const result = await tauriBackend.apply_action({kind:'check'});
    expect(result).toEqual(h);
    expect(fake.calls).toEqual([['apply_action', { action: {kind:'check'} }]]); }

  { const fake = new FakeBackend(); installMockIpc(fake); fake.hands.push(h);
    const result = await tauriBackend.set_board(['Kh','7d','2c']);
    expect(result).toEqual(h);
    expect(fake.calls).toEqual([['set_board', { board: ['Kh','7d','2c'] }]]); }

  { const fake = new FakeBackend(); installMockIpc(fake); fake.hands.push(h);
    const result = await tauriBackend.undo();
    expect(result).toEqual(h);
    expect(fake.calls).toEqual([['undo', {}]]); }

  { const fake = new FakeBackend(); installMockIpc(fake);
    const events: RecommendationEvent[] = [];
    const id = await tauriBackend.recommend(e => events.push(e));
    expect(id).toEqual(identity);
    expect(fake.calls).toEqual([['recommend', {}]]);
    expect(fake.sinks).toHaveLength(1); // the Channel was wired to exactly one fake sink
    const scripted: RecommendationEvent = {kind:'NoDecision', identity, reason:'contract-check'};
    fake.emit(scripted);
    expect(events).toEqual([scripted]); }

  { const fake = new FakeBackend(); installMockIpc(fake);
    await tauriBackend.cancel(identity.decision_id);
    expect(fake.calls).toEqual([['cancel', { decision_id: identity.decision_id }]]); }

  { const fake = new FakeBackend(); installMockIpc(fake);
    await tauriBackend.finish_hand();
    expect(fake.calls).toEqual([['finish_hand', {}]]); }

  { const fake = new FakeBackend(); installMockIpc(fake);
    await tauriBackend.abandon_hand();
    expect(fake.calls).toEqual([['abandon_hand', {}]]); }

  { const fake = new FakeBackend(); installMockIpc(fake);
    await expect(tauriBackend.set_seat_tag(2, 'nit')).rejects.toThrow(/phase 1/);
    expect(fake.calls).toEqual([['set_seat_tag', { seat: 2, tag: 'nit' }]]); }

  { const fake = new FakeBackend(); installMockIpc(fake);
    const status = await tauriBackend.presolver_status();
    expect(status).toEqual({paused:false,done:4,pending:7016,estimated_remaining_s:187200,scenario_hits:[['BTN-BB',1,10]]});
    expect(fake.calls).toEqual([['presolver_status', {}]]); }

  { const fake = new FakeBackend(); installMockIpc(fake);
    await tauriBackend.presolver_pause();
    expect(fake.calls).toEqual([['presolver_pause', {}]]); }

  { const fake = new FakeBackend(); installMockIpc(fake);
    await tauriBackend.presolver_resume();
    expect(fake.calls).toEqual([['presolver_resume', {}]]); }
});
