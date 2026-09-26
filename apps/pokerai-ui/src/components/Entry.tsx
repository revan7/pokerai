import {useId} from 'react';
import type {EntryState} from '../keys/handlers';
import {actionForKey,canWager} from '../keys/keymap';
import {decisionReason} from '../state/hand';
// Every control calls `press(key)`, the identical controller path keyboard entry takes; enabled
// state comes from the engine's `Derived.legal` via `actionForKey`/`canWager`, never from JS rules.
export function Entry({state:s,press}:{state:EntryState;press:(key:string)=>Promise<void>}){
  const errorId=useId();
  const w=s.wizard;
  // The board prompt appears only in AwaitingBoard, so a completed all-in runout never solicits one.
  const prompt=w?(w.step==='stack'?`Stack seat ${(w.dealt[w.index]??0)+1}: ${w.text} — unconfirmed; Enter confirms`:
    `${w.step}: ${w.text} ${w.step==='dealt'?w.dealt.map(seat=>seat+1).join(', '):''}`):
    s.mode==='hero'?`Hero cards: ${s.text}`:s.mode==='bet'?`Bet/raise TO (chips): ${s.text}`:
    s.hand?.phase.phase==='awaiting_board'?`Board cards: ${s.text} — Enter confirms`:'N new · H cards · F/C/A · B amount Enter · Ctrl+Z undo · Space recommend · E finish · X abandon · T tags';
  const button=(key:string,label:string,disabled:boolean)=><button key={key} disabled={disabled||s.busy}
    onClick={()=>{void press(key);}}>{label}</button>;
  // The error is field-level: it sits beside the entry field it concerns and describes it.
  return <section aria-label="Hand entry">
    <p role="status" data-testid="entry-prompt" aria-describedby={s.error?errorId:undefined}>{prompt}</p>
    {s.error&&<p id={errorId} role="alert">{s.error}</p>}
    {w&&<p>{w.confirmed.length}/{w.dealt.length} stacks confirmed</p>}
    {button('N','N New hand',false)}{button('H','H Hero cards',!s.hand)}
    {button('F','F Fold',!actionForKey(s.hand,'F'))}{button('C','C Check/call',!actionForKey(s.hand,'C'))}
    {button('A','A All-in',!actionForKey(s.hand,'A'))}{button('B','B Bet/raise to',!canWager(s.hand))}
    {button('Ctrl+Z','Undo',!s.hand&&!s.lastHand)}{button(' ','Recommend',decisionReason(s.hand)!==null)}
    {button('E','E Finish',!s.hand)}{button('X','X Abandon',!s.hand)}
    {s.mode==='tag'&&<aside aria-label="Seat tags"><p>Seat {(s.tagSeat??0)+1}: seat tags are unsupported in phase 1.</p><button onClick={()=>{void press('Escape');}}>Close tags</button></aside>}
  </section>;
}
