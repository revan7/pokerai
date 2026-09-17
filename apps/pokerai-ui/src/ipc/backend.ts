import { Channel, invoke } from '@tauri-apps/api/core';
import type { GameConfig, BeginHand, HandState, Card, Action, DecisionIdentity,
  RecommendationEvent, Seat, SeatTag } from './types.gen';

export interface Backend {
  set_game_config(config: GameConfig): Promise<GameConfig>;
  begin_hand(begin: BeginHand): Promise<HandState>;
  set_hero_cards(cards: [Card, Card]): Promise<HandState>;
  apply_action(action: Action): Promise<HandState>;
  set_board(board: Card[]): Promise<HandState>;
  undo(): Promise<HandState>;
  recommend(on_event: (event: RecommendationEvent) => void): Promise<DecisionIdentity>;
  cancel(decision_id: number): Promise<void>;
  finish_hand(): Promise<void>;
  abandon_hand(): Promise<void>;
  set_seat_tag(seat: Seat, tag: SeatTag): Promise<void>;
  presolver_status(): Promise<unknown>;
  presolver_pause(): Promise<void>;
  presolver_resume(): Promise<void>;
}

export const tauriBackend: Backend = {
  set_game_config: config => invoke('set_game_config', { config }),
  begin_hand: begin => invoke('begin_hand', { begin }),
  set_hero_cards: cards => invoke('set_hero_cards', { cards }),
  apply_action: action => invoke('apply_action', { action }),
  set_board: board => invoke('set_board', { board }),
  undo: () => invoke('undo'),
  recommend: on_event => {
    const channel = new Channel<RecommendationEvent>();
    channel.onmessage = on_event;
    return invoke('recommend', { on_event: channel });
  },
  cancel: decision_id => invoke('cancel', { decision_id }),
  finish_hand: () => invoke('finish_hand'),
  abandon_hand: () => invoke('abandon_hand'),
  set_seat_tag: (seat, tag) => invoke('set_seat_tag', { seat, tag }),
  presolver_status: () => invoke('presolver_status'),
  presolver_pause: () => invoke('presolver_pause'),
  presolver_resume: () => invoke('presolver_resume'),
};
