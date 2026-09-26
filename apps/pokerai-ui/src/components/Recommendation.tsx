import type {Recommendation,Action,ApproxReason,UnsupportedReason} from '../ipc/types.gen';
import type {DisplayState} from '../state/events';
import {AssumptionsPanel,describe} from './Assumptions';
import {Equity} from './Equity';import {Experimental} from './Experimental';
export function actionText(a:Action):string{return 'to' in a?`${a.kind} to ${a.to}`:a.kind;}
export function headline(r:Recommendation):{index:number;label:string}|null{
  // MI-13: the `coverage.kind==='Unsupported'` short-circuit is an added rule, not literally one
  // of spec §4.4's three headline rules — it is correct (Unsupported carries no numeric EV, and for
  // MultiwayEv carries no `actions` at all) but is recorded here so a later reader does not "fix"
  // it away by inlining the EV/frequency checks alone.
  if(r.unresolved_mass>0||r.actions.length===0||r.coverage.kind==='Unsupported')return null;
  const completeEv=r.actions.every(a=>a.ev_bb!==null),completeFreq=r.actions.every(a=>a.frequency!==null);
  let label:string;
  if(completeEv)label='highest EV';
  else if(!completeFreq)return null;
  else if(r.assumptions.source==='ChartTranscription')label='highest-frequency chart action';
  else if(r.assumptions.source==='PokerDataJson'&&r.coverage.kind==='Approximate'&&
    r.coverage.reasons.some(reason=>reason.kind==='EvReferenceUnverified'))label='highest-frequency source action, EV reference unverified';
  else if(r.actions.some(a=>a.unavailable?.kind==='BranchSupportIncomplete'))
    label='highest-frequency action, EV incomplete';
  else return null;
  const indices=r.actions.map((_,i)=>i);
  indices.sort((i,j)=>{const a=r.actions[i],b=r.actions[j];if(!a||!b)return i-j;
    return (completeEv?(b.ev_bb??0)-(a.ev_bb??0):0)||((b.frequency??-1)-(a.frequency??-1))||i-j;});
  const index=indices[0];return index===undefined?null:{index,label};
}
export function RecommendationPanel({display:d,fallbackReason}:{display:DisplayState;fallbackReason:string|null}){
  const noDecision=d.noDecision??fallbackReason;
  if(noDecision!==null)return <section aria-label="Recommendation"><p>no decision: {noDecision}</p></section>;
  const r=d.recommendation;
  const unsupported=r?.coverage.kind==='Unsupported';
  const label=r?.coverage.kind;
  // MA-2: the conditional's inferred type is a union of three distinct array types
  // (never[] | ApproxReason[] | (UnsupportedReason|ApproxReason)[]); `.map` on a union of more
  // than one generic-method array type is rejected (TS2349), so the binding needs an explicit
  // common element type instead of letting each branch infer its own.
  const reasons:Array<ApproxReason|UnsupportedReason>=!r||r.coverage.kind==='Exact'?[]:
    r.coverage.kind==='Approximate'?r.coverage.reasons:[r.coverage.reason,...r.coverage.partial];
  const head=r?headline(r):null;const chosen=r&&head?r.actions[head.index]:undefined;
  return <section aria-label="Recommendation" data-testid="recommendation" data-phase={r?.phase??'Waiting'}
    data-decision-id={d.active?.decision_id??''} data-requested-at={d.requestedAt??''} data-final-at={d.finalAt??''}>
    <h2>{r?.phase??'Waiting for recommendation'}</h2>
    {chosen&&head&&<h3>{head.label}: {actionText(chosen.action)}</h3>}
    {r&&<><p data-testid="coverage">{label}</p><ul>{reasons.map((reason,i)=><li key={i}
      data-prominent={'prominent' in reason&&reason.prominent===true}>{describe(reason)}</li>)}</ul>
      <p>Coverage describes input matching to the declared model.</p>
      {/* MI-14: spec §4.4 promises the recommendation's own legal intervals are "always present";
          Entry.tsx separately derives legality from HandState.derived.legal for gating buttons, but
          the recommendation's own answer was previously never rendered anywhere. */}
      <p data-testid="legal-intervals">Legal: {r.legal.map(describe).join(' · ')}</p>
      <table aria-label="Hero actions"><thead><tr><th>Action</th><th>Frequency</th><th>EV (bb)</th><th>Unavailable reason</th></tr></thead><tbody>
        {r.actions.map((a,i)=><tr key={i}><th>{actionText(a.action)}</th>
          <td>{a.frequency===null?'unavailable':`${(a.frequency*100).toFixed(1)}%`}</td>
          <td>{unsupported||a.ev_bb===null?'unavailable':<span data-testid="main-ev" data-ev-bb={a.ev_bb}>{a.ev_bb.toFixed(2)} bb</span>}</td>
          <td>{a.unavailable===null?'':describe(a.unavailable)}</td></tr>)}
      </tbody></table>
      {r.unresolved_mass>0&&<p>{(100*r.unresolved_mass).toFixed(1)}% of the posterior has no strategy</p>}
      {r.range_mix&&<section aria-label="Range-level mix"><h3>Range-level mix</h3>{r.range_mix.map(([a,f],i)=><p key={i}>{actionText(a)}: {(100*f).toFixed(1)}%</p>)}</section>}
      <AssumptionsPanel value={r.assumptions}/></>}
    {d.progress&&<p data-testid="progress">{d.progress.stage} · {d.progress.iterations} iterations · {
      d.progress.exploitability_pct===null?'measuring':`${d.progress.exploitability_pct.toFixed(2)}% pot`} · {(d.progress.elapsed_ms/1000).toFixed(1)} s</p>}
    <Equity value={d.equity}/>{r?.experimental&&<Experimental value={r.experimental}/>}
  </section>;
}
