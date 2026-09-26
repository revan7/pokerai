import type {Assumptions} from '../ipc/types.gen';
export function describe(value:unknown):string{return typeof value==='string'?value:JSON.stringify(value)??String(value);}
export function AssumptionsPanel({value:a}:{value:Assumptions}){
  return <details open><summary>Assumptions</summary>
    <p>source: {a.source} · source_accuracy: {a.source_accuracy} · source_granularity: {a.source_granularity}</p>
    <p>template: {a.template_id} · tree: {a.tree_signature}</p>
    <p>target: {a.target_bp} bp · reached: {a.reached_bp===null?'unmeasured':`${a.reached_bp} bp`} · elapsed: {a.elapsed_ms} ms · cache: {a.cache}</p>
    <h4>Ranges used</h4><ul>{a.ranges_used.map(([seat,range,mass],i)=><li key={i}>Seat {seat+1}: {range}; mass {mass}</li>)}</ul>
    <h4>Translations</h4><ul>{a.translations.map((r,i)=><li key={i}>{describe(r)}</li>)}</ul>
    <h4>Mappings</h4><ul>{a.mappings.map((r,i)=><li key={i}>{describe(r)}</li>)}</ul>
    <h4>Notes</h4><ul>{a.notes.map((note,i)=><li key={i}>{note}</li>)}</ul>
  </details>;
}
