import type { GameConfig,Seat } from '../ipc/types.gen';
export type StackDrafts=Record<number,number>;
export function presolverLines(status:unknown):string[]{
  if(typeof status!=='object'||status===null)return ['Loading status…'];
  const data=status as Record<string,unknown>;
  // MA-1: `key` is a plain `string`, not a literal type, so TS performs no narrowing on
  // `data[key]` — return an explicit `number|null` instead of the bare (and unnarrowed) element
  // access, so every call site below gets a real `number`, not `unknown`.
  const num=(key:string):number|null=>{const v=data[key];return typeof v==='number'&&Number.isFinite(v)?v:null;};
  const eta=num('estimated_remaining_s'),p50=num('measured_p50_s');
  const output=[
    data.paused===true?'Paused':'Enabled',
    `Running: ${typeof data.running==='string'?data.running:'none'}`,
    `Done: ${num('done')??0} · Pending: ${num('pending')??0} · Failed: ${num('failed')??0}`,
    `Estimated remaining: ${eta===null?'measuring':(eta/3600).toFixed(1)+' hours'}`,
    `Measured median: ${p50===null?'measuring':p50.toFixed(1)+' seconds'}`,
  ];
  // MA-1: `Array.isArray(data.tier_done)` narrows `data.tier_done` to `any[]`, so indexing it
  // binds `any` under `no-unsafe-assignment`. Narrow into a typed local via an explicit `unknown[]`
  // cast instead, then re-check each element's type before use.
  const list=(key:string):unknown[]=>Array.isArray(data[key])?data[key] as unknown[]:[];
  // MA-1: `tierDone[i]` is `unknown` under `noUncheckedIndexedAccess`, and `unknown??0` stays
  // `unknown` (`??` cannot narrow away `null`/`undefined` from an `unknown` operand), so even a
  // wrapping `String(...)` call trips `no-base-to-string`. Re-check each element's type before
  // stringifying it, per this function's own MA-1 rule above.
  const cell=(arr:unknown[],i:number):number|string=>{const v=arr[i];return typeof v==='number'||typeof v==='string'?v:0;};
  const tierDone=list('tier_done'),tierTotal=list('tier_total');
  if(tierDone.length>0&&tierTotal.length>0){
    for(let i=0;i<3;i++)output.push(`Tier ${i+1}: ${cell(tierDone,i)} / ${cell(tierTotal,i)}`);
  }
  for(const row of list('scenario_hits')){
    if(!Array.isArray(row))continue;
    const r=row as unknown[];const name=r[0],hit=r[1],total=r[2];
    if(typeof name==='string'&&typeof hit==='number'&&typeof total==='number')
      output.push(`Cache hits, ${name}: ${hit} / ${total} (${total>0?(100*hit/total).toFixed(1):'0.0'}%)`);
  }
  return output;
}
function text(form:FormData,key:string):string {const value=form.get(key);return typeof value==='string'?value.trim():'';}
function chips(value:string,name:string):number {
  if(!/^\d+$/.test(value)) throw new Error(`${name} must be whole chips`);
  const n=Number(value);if(!Number.isSafeInteger(n)||n>2_147_483_647) throw new Error(`${name} is too large`);
  return n;
}
export function readSession(form:FormData,previous:GameConfig):{config:GameConfig;stacks:StackDrafts} {
  const sb=chips(text(form,'sb_chips'),'Small blind'),bb=chips(text(form,'bb_chips'),'Big blind');
  if(sb<=0||bb<sb) throw new Error('Blinds must be positive; big blind must be at least the small blind');
  const seats=form.getAll('seat').map(v=>Number(v));
  if(seats.length<3||seats.length>6||new Set(seats).size!==seats.length||seats.some(s=>s<0||s>5||!Number.isInteger(s)))
    throw new Error('Select 3 to 6 seats');
  const str=text(form,'straddle');const amount=str===''?null:chips(str,'UTG straddle');
  if(amount!==null&&(amount<2*bb||seats.length!==6)) throw new Error('UTG straddle requires six seats and at least twice the big blind');
  const budget=Number(text(form,'flop_budget_s'));
  if(!Number.isInteger(budget)||budget<1||budget>30) throw new Error('Flop budget must be 1 to 30 seconds');
  const stacks:StackDrafts={};let total=0;
  // Deviation from the brief's literal sample (see task-8-report.md): a blank stack-<seat>
  // field defaults to 100 big blinds, matching ConfigScreen's own `defaultValue={stacks[seat]
  // ??100*c.bb_chips}` -- the brief's own edge-case fixture omits every stack-<seat> field, so
  // treating a blank stack as a required chip amount (like blinds) makes that fixture throw
  // before it ever reaches the rake/straddle assertions it is actually testing.
  for(const seat of seats){const raw=text(form,`stack-${seat}`);const n=raw===''?100*bb:chips(raw,'Stack');stacks[seat]=n;total+=n;}
  if(total>=2_147_483_648) throw new Error('Total chips must be less than 2^31');
  const rate=Number(text(form,'rate'))/100;
  const cap=text(form,'cap');const parts=/^(\d+)(?:\.(\d{1,3}))?$/.exec(cap);
  const time=text(form,'rake')==='TimeCharge';
  if(!time&&(!Number.isFinite(rate)||rate<0||rate>1||!parts)) throw new Error('Rake requires 0 to 100 percent and a cap with at most three decimals');
  const capM=time?0:parts?Number(parts[1])*1000+Number((parts[2]??'').padEnd(3,'0')):0;
  if(!Number.isSafeInteger(capM)||capM>4_294_967_295) throw new Error('Rake cap is too large');
  return {stacks,config:{...previous,chip_label:text(form,'chip_label'),sb_chips:sb,bb_chips:bb,
    straddle:amount===null?null:{amount_chips:amount},
    rake:time?{kind:'time_charge'}:{kind:'pot_rake',rate,cap_mchips:capM,no_flop_no_drop:form.has('no_flop_no_drop')},
    seats:seats.map((seat:Seat)=>previous.seats.find(s=>s.seat===seat)??{seat,tag:null,facts:[]}),
    solver:{...previous.solver,flop_budget_s:budget}}};
}
