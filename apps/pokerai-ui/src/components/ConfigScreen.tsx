import { useRef,useState } from 'react';
import type { GameConfig } from '../ipc/types.gen';
import { describeError,readSession } from '../state/session';
import type { StackDrafts } from '../state/session';
type Props={config:GameConfig;stacks:StackDrafts;activeHand:boolean;
  save:(config:GameConfig,stacks:StackDrafts)=>Promise<void>;onError:(e:unknown)=>void};
export function ConfigScreen({config:c,stacks,activeHand,save,onError}:Props){
  const rake=c.rake.kind==='pot_rake'?c.rake:null;
  // R2 (fix round 1): validation failures -- local or a rejected `save` -- render beside the
  // field they concern instead of only reaching a single generic `onError` callback; `formError`
  // is the explicit fallback for a failure `describeError` cannot associate with one field.
  const [fieldErrors,setFieldErrors]=useState<Record<string,string>>({});
  const [formError,setFormError]=useState<string|null>(null);
  // R4 (fix round 1): the straddle minimum must track the big-blind draft on every render, not the
  // configuration this screen was opened with, so a same-submission "lower BB, raise straddle to
  // just above the new BB" edit is never blocked by a stale native `min`.
  const [bbDraft,setBbDraft]=useState(c.bb_chips);
  const fieldRefs=useRef<Record<string,HTMLElement|null>>({});
  const registerRef=(key:string)=>(el:HTMLElement|null)=>{fieldRefs.current[key]=el;};
  function applyError(error:unknown){
    const {field,message}=describeError(error);
    if(field!==null){setFieldErrors(prev=>({...prev,[field]:message}));fieldRefs.current[field]?.focus();}
    else{setFormError(message);}
    onError(error);
  }
  function errorProps(field:string):{'aria-invalid':boolean;'aria-describedby'?:string} {
    const message=fieldErrors[field];
    return message===undefined?{'aria-invalid':false}:{'aria-invalid':true,'aria-describedby':`${field}-error`};
  }
  function errorText(field:string){
    const message=fieldErrors[field];
    return message===undefined?null:<p id={`${field}-error`} role="alert">{message}</p>;
  }
  return <section aria-label="Session configuration"><h2>Session configuration</h2>
    {activeHand&&<p>Changes apply to the next hand.</p>}
    {formError!==null&&<p role="alert">{formError}</p>}
    <form data-testid="config-form" onSubmit={e=>{e.preventDefault();
      setFieldErrors({});setFormError(null);
      try{
        const result=readSession(new FormData(e.currentTarget),c);
        void save(result.config,result.stacks).catch(applyError);
      }catch(error){applyError(error);}
    }}>
      <label>Chip value<input name="chip_label" defaultValue={c.chip_label} required
        ref={registerRef('chip_label')} {...errorProps('chip_label')}/></label>
      {errorText('chip_label')}
      <label>Small blind<input name="sb_chips" type="number" min="1" step="1" defaultValue={c.sb_chips} required
        ref={registerRef('sb_chips')} {...errorProps('sb_chips')}/></label>
      {errorText('sb_chips')}
      <label>Big blind<input name="bb_chips" type="number" min="1" step="1" defaultValue={c.bb_chips} required
        ref={registerRef('bb_chips')} {...errorProps('bb_chips')}
        onChange={e=>{const n=Number(e.target.value);setBbDraft(Number.isFinite(n)&&n>0?n:c.bb_chips);}}/></label>
      {errorText('bb_chips')}
      <label>UTG straddle (blank for none)<input name="straddle" type="number" min={2*bbDraft} step="1"
        defaultValue={c.straddle?.amount_chips??''} ref={registerRef('straddle')} {...errorProps('straddle')}/></label>
      {errorText('straddle')}
      <label>Charge<select name="rake" defaultValue={rake?'PotRake':'TimeCharge'}><option value="PotRake">Pot rake</option><option value="TimeCharge">Time charge — solved unraked</option></select></label>
      {/* R1 (fix round 1): the wire domain is half-open `[0, 1)`, i.e. percent `[0, 100)` -- an
          inclusive native `max="100"` would let the browser accept exactly 100% (and, on some
          browsers, silently clamp an out-of-range typed value on blur/step instead of failing
          validation), so the upper bound is enforced only in `readSession`, never natively. */}
      <label>Rake (%)<input name="rate" type="number" min="0" step="0.001" defaultValue={(rake?.rate??0)*100}
        ref={registerRef('rate')} {...errorProps('rate')}/></label>
      {errorText('rate')}
      <label>Rake cap (chips)<input name="cap" inputMode="decimal" defaultValue={(rake?.cap_mchips??0)/1000}
        ref={registerRef('cap')} {...errorProps('cap')}/></label>
      {errorText('cap')}
      <label><input name="no_flop_no_drop" type="checkbox" defaultChecked={rake?.no_flop_no_drop??true}/>No flop, no drop</label>
      <fieldset tabIndex={-1} ref={registerRef('seat')}><legend>Seats and starting stack drafts</legend>
      {errorText('seat')}
      {errorText('stacks')}
      {[0,1,2,3,4,5].map(seat=><div key={seat}>
        <label><input name="seat" type="checkbox" value={seat} defaultChecked={c.seats.some(s=>s.seat===seat)}/>Seat {seat+1}</label>
        {/* No native `required`: a blank stack must reach `readSession` so its own field-specific
            "Stack for seat N is required" message renders (R3), rather than being intercepted by
            the browser's native constraint validation before `onSubmit` ever runs. */}
        <label>Stack for seat {seat+1}<input name={`stack-${seat}`} type="number" min="0" step="1"
          defaultValue={stacks[seat]??100*c.bb_chips} ref={registerRef(`stack-${seat}`)} {...errorProps(`stack-${seat}`)}/></label>
        {errorText(`stack-${seat}`)}
      </div>)}</fieldset>
      <label>Flop budget (seconds)<input name="flop_budget_s" type="number" min="1" max="30" step="1" defaultValue={c.solver.flop_budget_s} required
        ref={registerRef('flop_budget_s')} {...errorProps('flop_budget_s')}/></label>
      {errorText('flop_budget_s')}
      <p>Default 10 s; maximum 30 s. Flop final delivery is 5 s plus this budget. Turn and river remain 15 s.</p>
      <button type="submit">Save session</button>
    </form></section>;
}
