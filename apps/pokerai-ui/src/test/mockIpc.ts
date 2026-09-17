import { mockIPC } from '@tauri-apps/api/mocks';
import { Channel } from '@tauri-apps/api/core';
import type { FakeBackend } from './fakeBackend';
import type { GameConfig, BeginHand, Card, Action, Seat, SeatTag } from '../ipc/types.gen';

export function installMockIpc(fake: FakeBackend) {
  mockIPC((command, payload) => {
    const args = payload as Record<string, unknown> | undefined;
    switch (command) {
      case 'set_game_config': return fake.set_game_config(args?.config as GameConfig);
      case 'begin_hand': return fake.begin_hand(args?.begin as BeginHand);
      case 'set_hero_cards': return fake.set_hero_cards(args?.cards as [Card,Card]);
      case 'apply_action': return fake.apply_action(args?.action as Action);
      case 'set_board': return fake.set_board(args?.board as Card[]);
      case 'undo': return fake.undo();
      case 'recommend': {
        const channel = args?.on_event;
        if (!(channel instanceof Channel)) throw new Error('recommend requires on_event Channel');
        return fake.recommend(e => channel.onmessage(e));
      }
      case 'cancel': return fake.cancel(args?.decision_id as number);
      case 'finish_hand': return fake.finish_hand();
      case 'abandon_hand': return fake.abandon_hand();
      case 'set_seat_tag': return fake.set_seat_tag(args?.seat as Seat,args?.tag as SeatTag);
      case 'presolver_status': return fake.presolver_status();
      case 'presolver_pause': return fake.presolver_pause();
      case 'presolver_resume': return fake.presolver_resume();
      default: throw new Error(`Unexpected IPC command: ${command}`);
    }
  });
}
