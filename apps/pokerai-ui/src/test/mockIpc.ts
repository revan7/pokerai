import { mockIPC } from '@tauri-apps/api/mocks';
import { Channel } from '@tauri-apps/api/core';
import type { FakeBackend } from './fakeBackend';
import type { GameConfig, BeginHand, Card, Action, Seat, SeatTag } from '../ipc/types.gen';

// Runtime wire-shape guards (R1, review round 1). Type assertions perform no runtime
// validation, so a UI transport bug (missing/renamed key, wrong scalar or collection
// type, malformed card text, an unknown action/tag discriminant) would otherwise pass
// through this mock silently, even though Rust's `serde` deserialization rejects it
// before engine dispatch (see `src-tauri/src/tests.rs:132`, the invalid-card-text
// case). Every argument-bearing command below is validated before it ever reaches
// the fake, so a mismatch fails loudly here with a descriptive error, and the fake
// never records a call or consumes a scripted snapshot for a rejected payload.

function isRecord(x: unknown): x is Record<string, unknown> {
  return typeof x === 'object' && x !== null && !Array.isArray(x);
}
function isFiniteNumber(x: unknown): x is number {
  return typeof x === 'number' && Number.isFinite(x);
}
function isString(x: unknown): x is string {
  return typeof x === 'string';
}
// Wire format: exactly two characters, a rank from "23456789TJQKA" then a suit from
// "cdhs" (crates/proto/src/cards.rs RANK_CHARS/SUIT_CHARS), case-insensitive to match
// `Card::from_str`'s acceptance.
function isCard(x: unknown): x is Card {
  return typeof x === 'string' && /^[2-9TJQKA][CDHS]$/i.test(x);
}
function isCardArray(x: unknown): x is Card[] {
  return Array.isArray(x) && x.every(isCard);
}
function isCardPair(x: unknown): x is [Card, Card] {
  return Array.isArray(x) && x.length === 2 && isCard(x[0]) && isCard(x[1]);
}
function isSeat(x: unknown): x is Seat {
  return isFiniteNumber(x) && Number.isInteger(x) && x >= 0;
}
function isSeatArray(x: unknown): x is Seat[] {
  return Array.isArray(x) && x.every(isSeat);
}
function isNumberArray(x: unknown): x is number[] {
  return Array.isArray(x) && x.every(isFiniteNumber);
}
const ACTION_KINDS = ['fold', 'check', 'call', 'bet', 'raise', 'allin'] as const;
const ACTION_KINDS_WITH_TO = new Set<string>(['bet', 'raise', 'allin']);
function isAction(x: unknown): x is Action {
  if (!isRecord(x) || !isString(x.kind)) return false;
  if (!(ACTION_KINDS as readonly string[]).includes(x.kind)) return false;
  if (ACTION_KINDS_WITH_TO.has(x.kind)) return isFiniteNumber(x.to);
  return true;
}
const SEAT_TAGS = ['unknown', 'nit', 'tag', 'loose_passive', 'calling_station', 'lag', 'maniac'] as const;
function isSeatTag(x: unknown): x is SeatTag {
  return typeof x === 'string' && (SEAT_TAGS as readonly string[]).includes(x);
}
function isGameConfig(x: unknown): x is GameConfig {
  return isRecord(x) && isFiniteNumber(x.config_revision) && isString(x.chip_label)
    && isFiniteNumber(x.sb_chips) && isFiniteNumber(x.bb_chips) && isRecord(x.rake)
    && Array.isArray(x.seats) && isRecord(x.solver);
}
function isBeginHand(x: unknown): x is BeginHand {
  if (!isRecord(x) || !isSeat(x.button) || !isSeat(x.hero)) return false;
  if (!isSeatArray(x.dealt) || !isNumberArray(x.stacks)) return false;
  return x.hero_cards === null || isCardPair(x.hero_cards);
}

function fail(command: string, detail: string): never {
  throw new Error(`Invalid arguments for "${command}": ${detail}`);
}

export function installMockIpc(fake: FakeBackend) {
  mockIPC((command, payload) => {
    const args = payload as Record<string, unknown> | undefined;
    switch (command) {
      case 'set_game_config': {
        if (!isGameConfig(args?.config)) fail(command, 'expected { config: GameConfig }');
        return fake.set_game_config(args.config);
      }
      case 'begin_hand': {
        if (!isBeginHand(args?.begin)) fail(command, 'expected { begin: BeginHand }');
        return fake.begin_hand(args.begin);
      }
      case 'set_hero_cards': {
        if (!isCardPair(args?.cards)) fail(command, 'expected { cards: [Card, Card] }');
        return fake.set_hero_cards(args.cards);
      }
      case 'apply_action': {
        if (!isAction(args?.action)) fail(command, 'expected { action: Action }');
        return fake.apply_action(args.action);
      }
      case 'set_board': {
        if (!isCardArray(args?.board)) fail(command, 'expected { board: Card[] }');
        return fake.set_board(args.board);
      }
      case 'undo': return fake.undo();
      case 'recommend': {
        const channel = args?.on_event;
        if (!(channel instanceof Channel)) fail(command, 'expected { on_event: Channel<RecommendationEvent> }');
        return fake.recommend(e => channel.onmessage(e));
      }
      case 'cancel': {
        if (!isFiniteNumber(args?.decision_id)) fail(command, 'expected { decision_id: number }');
        return fake.cancel(args.decision_id);
      }
      case 'finish_hand': return fake.finish_hand();
      case 'abandon_hand': return fake.abandon_hand();
      case 'set_seat_tag': {
        if (!isSeat(args?.seat) || !isSeatTag(args?.tag)) fail(command, 'expected { seat: Seat, tag: SeatTag }');
        return fake.set_seat_tag(args.seat, args.tag);
      }
      case 'presolver_status': return fake.presolver_status();
      case 'presolver_pause': return fake.presolver_pause();
      case 'presolver_resume': return fake.presolver_resume();
      default: throw new Error(`Unexpected IPC command: ${command}`);
    }
  });
}
