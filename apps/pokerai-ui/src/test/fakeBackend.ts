import type { Backend } from '../ipc/backend';
import type { HandState, GameConfig, BeginHand, Card, Action, Seat, SeatTag,
  RecommendationEvent, DecisionIdentity } from '../ipc/types.gen';
import { identity } from './fixtures';

export class FakeBackend implements Backend {
  calls: Array<[string, unknown]> = [];
  hands: HandState[] = [];
  sinks: Array<(e: RecommendationEvent) => void> = [];
  nextIdentity: DecisionIdentity = identity;
  beforeIdentity: RecommendationEvent[] = [];
  private record(name: string, args: unknown = {}) { this.calls.push([name,args]); }
  private next(name: string, args: unknown): Promise<HandState> {
    this.record(name,args); const value = this.hands.shift();
    return value ? Promise.resolve(value) : Promise.reject(new Error(`Unscripted ${name}`));
  }
  set_game_config(config: GameConfig) { this.record('set_game_config',{config}); return Promise.resolve({...config,config_revision:config.config_revision+1}); }
  begin_hand(begin: BeginHand) { return this.next('begin_hand',{begin}); }
  set_hero_cards(cards: [Card,Card]) { return this.next('set_hero_cards',{cards}); }
  apply_action(action: Action) { return this.next('apply_action',{action}); }
  set_board(board: Card[]) { return this.next('set_board',{board}); }
  undo() { return this.next('undo',{}); }
  recommend(sink: (e: RecommendationEvent) => void) {
    this.record('recommend'); this.sinks.push(sink);
    for (const event of this.beforeIdentity) sink(event);
    return Promise.resolve(this.nextIdentity);
  }
  emit(e: RecommendationEvent, request = this.sinks.length-1) { this.sinks[request]?.(e); }
  cancel(decision_id: number) { this.record('cancel',{decision_id}); return Promise.resolve(); }
  finish_hand() { this.record('finish_hand'); return Promise.resolve(); }
  abandon_hand() { this.record('abandon_hand'); return Promise.resolve(); }
  set_seat_tag(seat: Seat, tag: SeatTag) { this.record('set_seat_tag',{seat,tag}); return Promise.reject(new Error('unsupported in phase 1')); }
  presolver_status() { this.record('presolver_status'); return Promise.resolve({paused:false,done:4,pending:7016,estimated_remaining_s:187200,scenario_hits:[['BTN-BB',1,10]]}); }
  presolver_pause() { this.record('presolver_pause'); return Promise.resolve(); }
  presolver_resume() { this.record('presolver_resume'); return Promise.resolve(); }
}
