import type {ExperimentalHu} from '../ipc/types.gen';
export function Experimental({value:e}:{value:ExperimentalHu}){
  return <aside aria-label="experimental, not solved" className="experimental"><h3>experimental, not solved</h3>
    <p>{e.note}</p><p>Opponent seat {e.opponent+1}; hero {e.hero_role}; synthetic pot {e.pot} chips; stack {e.stack} chips</p>
    <p>Template {e.template_id}; reached {e.reached_bp??'unmeasured'} bp; elapsed {e.elapsed_ms} ms</p>
    <ul>{e.ranges_used.map(([seat,range,mass])=><li key={seat}>Street-root public range, seat {seat+1}: {range}; mass {mass}</li>)}</ul>
    <table aria-label="Experimental actions"><tbody>{e.actions.map((a,i)=><tr key={i}>
      <th>{a.action.kind}{'to' in a.action?` to ${a.action.to}`:''}</th>
      <td>{a.frequency===null?'unavailable':`${(a.frequency*100).toFixed(1)}%`}</td>
      <td>{a.ev_bb===null?'unavailable':`${a.ev_bb.toFixed(2)} bb (experimental)`}</td>
    </tr>)}</tbody></table></aside>;
}
