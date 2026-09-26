import type {EquityEstimate,EquitySummary} from '../ipc/types.gen';
function Estimate({value:e}:{value:EquityEstimate}){
  if(e.availability.kind==='Pending')return <span>pending</span>;
  if(e.availability.kind==='Unavailable')return <span>unavailable: {e.availability.reason}</span>;
  if(e.value===null||e.method===null)return <span>unavailable</span>;
  return <span>{(100*e.value).toFixed(2)}% · {e.method.kind==='Exact'?'Exact':
    `MonteCarlo; samples ${e.method.samples}; std error ${(100*e.method.std_err).toFixed(3)} pp`}</span>;
}
export function Equity({value:e}:{value:EquitySummary}){
  return <section aria-label="Equity"><h3>Equity</h3>
    <h4>Hero combo versus each opponent</h4>{e.hero_combo_vs_each.map(([seat,value])=><p key={seat}>Seat {seat+1}: <Estimate value={value}/></p>)}
    <h4>Hero public range versus each public range</h4>{e.hero_range_vs_each.map(([seat,value])=><p key={seat}>Seat {seat+1}: <Estimate value={value}/></p>)}
    {e.per_pot_shares.map(p=><section key={p.pot_index} aria-label={`Pot ${p.pot_index+1} equity`}>
      <p>{p.population}</p>{p.shares.map(([seat,value])=><p key={seat}>Seat {seat+1}: <Estimate value={value}/></p>)}
    </section>)}</section>;
}
