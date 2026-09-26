import type {HandState} from '../ipc/types.gen';
import {cardText} from '../state/hand';
export function Table({hand:h}:{hand:HandState|null}){
  if(!h)return <section aria-label="Table"><p>No hand. Press N.</p></section>;
  return <section aria-label="Table" data-testid="table" data-hand-id={h.hand_id} data-hand-revision={h.hand_revision}>
    <p data-testid="seat-to-act">{h.derived.to_act===null?'No seat to act':`Seat ${h.derived.to_act+1} to act`}</p>
    <p>Pot: {h.derived.pot} chips · Blinds: {h.config.sb_chips}/{h.config.bb_chips} · {h.derived.street}</p>
    <p>Hero: {h.hero_cards?.map(cardText).join(' ')??'unknown'} · Board: {h.board.map(cardText).join(' ')||'none'}</p>
    <table><thead><tr><th>Seat</th><th>Stack</th><th>Street contribution</th><th>Status</th></tr></thead><tbody>
      {h.dealt.map(seat=><tr key={seat} aria-current={h.derived.to_act===seat?'true':undefined}>
        <th>Seat {seat+1}{seat===h.button?' BTN':''}{seat===h.hero?' Hero':''}</th>
        <td>{h.derived.stacks_remaining[seat]}</td><td>{h.derived.committed_this_street[seat]}</td>
        <td>{h.derived.folded[seat]?'folded':h.derived.all_in[seat]?'all-in':'live'}</td></tr>)}
    </tbody></table><p data-testid="hand-phase">{JSON.stringify(h.phase)}</p></section>;
}
