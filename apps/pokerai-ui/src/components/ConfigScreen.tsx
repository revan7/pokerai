import type { GameConfig } from '../ipc/types.gen';
import { readSession } from '../state/session';
import type { StackDrafts } from '../state/session';
type Props={config:GameConfig;stacks:StackDrafts;activeHand:boolean;
  save:(config:GameConfig,stacks:StackDrafts)=>Promise<void>;onError:(e:unknown)=>void};
export function ConfigScreen({config:c,stacks,activeHand,save,onError}:Props){
  const rake=c.rake.kind==='pot_rake'?c.rake:null;
  return <section aria-label="Session configuration"><h2>Session configuration</h2>
    {activeHand&&<p>Changes apply to the next hand.</p>}
    <form data-testid="config-form" onSubmit={e=>{e.preventDefault();try{
      const result=readSession(new FormData(e.currentTarget),c);void save(result.config,result.stacks).catch(onError);
    }catch(error){onError(error);}}}>
      <label>Chip value<input name="chip_label" defaultValue={c.chip_label} required/></label>
      <label>Small blind<input name="sb_chips" type="number" min="1" step="1" defaultValue={c.sb_chips} required/></label>
      <label>Big blind<input name="bb_chips" type="number" min="1" step="1" defaultValue={c.bb_chips} required/></label>
      <label>UTG straddle (blank for none)<input name="straddle" type="number" min={2*c.bb_chips} step="1" defaultValue={c.straddle?.amount_chips??''}/></label>
      <label>Charge<select name="rake" defaultValue={rake?'PotRake':'TimeCharge'}><option value="PotRake">Pot rake</option><option value="TimeCharge">Time charge — solved unraked</option></select></label>
      <label>Rake (%)<input name="rate" type="number" min="0" max="100" step="0.001" defaultValue={(rake?.rate??0)*100}/></label>
      <label>Rake cap (chips)<input name="cap" inputMode="decimal" defaultValue={(rake?.cap_mchips??0)/1000}/></label>
      <label><input name="no_flop_no_drop" type="checkbox" defaultChecked={rake?.no_flop_no_drop??true}/>No flop, no drop</label>
      <fieldset><legend>Seats and starting stack drafts</legend>{[0,1,2,3,4,5].map(seat=><div key={seat}>
        <label><input name="seat" type="checkbox" value={seat} defaultChecked={c.seats.some(s=>s.seat===seat)}/>Seat {seat+1}</label>
        <label>Stack for seat {seat+1}<input name={`stack-${seat}`} type="number" min="0" step="1" defaultValue={stacks[seat]??100*c.bb_chips}/></label>
      </div>)}</fieldset>
      <label>Flop budget (seconds)<input name="flop_budget_s" type="number" min="1" max="30" step="1" defaultValue={c.solver.flop_budget_s} required/></label>
      <p>Default 10 s; maximum 30 s. Flop final delivery is 5 s plus this budget. Turn and river remain 15 s.</p>
      <button type="submit">Save session</button>
    </form></section>;
}
