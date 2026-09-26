import type { GameConfig,Seat } from '../ipc/types.gen';
export type StackDrafts=Record<number,number>;

/** A validation failure scoped to one form field (review R2, fix round 1): carrying the field key
 *  lets `ConfigScreen` render the message beside that field and associate it via
 *  `aria-describedby`, instead of a single generic error for the whole form. */
export class FieldError extends Error {
  readonly field:string;
  constructor(field:string,message:string){super(message);this.name='FieldError';this.field=field;}
}

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
function chips(value:string,name:string,field:string):number {
  if(!/^\d+$/.test(value)) throw new FieldError(field,`${name} must be whole chips`);
  const n=Number(value);if(!Number.isSafeInteger(n)||n>2_147_483_647) throw new FieldError(field,`${name} is too large`);
  return n;
}
export function readSession(form:FormData,previous:GameConfig):{config:GameConfig;stacks:StackDrafts} {
  const sb=chips(text(form,'sb_chips'),'Small blind','sb_chips'),bb=chips(text(form,'bb_chips'),'Big blind','bb_chips');
  if(sb<=0) throw new FieldError('sb_chips','Small blind must be positive');
  if(bb<sb) throw new FieldError('bb_chips','Big blind must be at least the small blind');
  const seats=form.getAll('seat').map(v=>Number(v));
  if(seats.length<3||seats.length>6||new Set(seats).size!==seats.length||seats.some(s=>s<0||s>5||!Number.isInteger(s)))
    throw new FieldError('seat','Select 3 to 6 seats');
  const str=text(form,'straddle');const amount=str===''?null:chips(str,'UTG straddle','straddle');
  if(amount!==null&&(amount<2*bb||seats.length!==6))
    throw new FieldError('straddle','UTG straddle requires six seats and at least twice the big blind');
  const budget=Number(text(form,'flop_budget_s'));
  if(!Number.isInteger(budget)||budget<1||budget>30) throw new FieldError('flop_budget_s','Flop budget must be 1 to 30 seconds');
  const stacks:StackDrafts={};let total=0;
  // R3 (fix round 1, orchestrator ruling): a blank stack is an explicit field error, never an
  // implicit default. `ConfigScreen` always renders a visible non-blank starting value (the
  // existing stack, or 100 big blinds) for every dealt seat, so a blank field here can only be one
  // the user deliberately cleared -- and clearing it must surface as a required-field error, not
  // silently resurrect the 100-BB default underneath the visibly blank input.
  for(const seat of seats){
    const raw=text(form,`stack-${seat}`);
    if(raw==='') throw new FieldError(`stack-${seat}`,`Stack for seat ${seat+1} is required`);
    const n=chips(raw,'Stack',`stack-${seat}`);stacks[seat]=n;total+=n;
  }
  if(total>=2_147_483_648) throw new FieldError('stacks','Total chips must be less than 2^31');
  const rate=Number(text(form,'rate'))/100;
  const cap=text(form,'cap');const parts=/^(\d+)(?:\.(\d{1,3}))?$/.exec(cap);
  const time=text(form,'rake')==='TimeCharge';
  // R1 (fix round 1): mirror `crates/proto/src/numeric.rs`'s `domain_rake_rate`, `[0, 1)` --
  // half-open, so exactly 100% (`rate === 1`) is rejected, not just values above it -- and its
  // wide-before-narrow check: a wide (f64) rate that is inside the domain can still narrow to an
  // out-of-domain `f32` (e.g. `0.99999999` narrows to exactly `1`), and that must be rejected too,
  // without ever substituting the narrowed value into the draft (standing ruling (a)).
  if(!time){
    if(!Number.isFinite(rate)||rate<0||rate>=1)
      throw new FieldError('rate','Rake requires 0 up to (but not including) 100 percent');
    const narrowedRate=Math.fround(rate);
    if(!(narrowedRate>=0&&narrowedRate<1))
      throw new FieldError('rate','Rake is too close to 100 percent once rounded; enter a slightly lower rate');
    if(!parts) throw new FieldError('cap','Rake cap must have at most three decimal places');
  }
  const capM=time?0:parts?Number(parts[1])*1000+Number((parts[2]??'').padEnd(3,'0')):0;
  if(!Number.isSafeInteger(capM)||capM>4_294_967_295) throw new FieldError('cap','Rake cap is too large');
  return {stacks,config:{...previous,chip_label:text(form,'chip_label'),sb_chips:sb,bb_chips:bb,
    straddle:amount===null?null:{amount_chips:amount},
    rake:time?{kind:'time_charge'}:{kind:'pot_rake',rate,cap_mchips:capM,no_flop_no_drop:form.has('no_flop_no_drop')},
    seats:seats.map((seat:Seat)=>previous.seats.find(s=>s.seat===seat)??{seat,tag:null,facts:[]}),
    solver:{...previous.solver,flop_budget_s:budget}}};
}

/** Recognizes the Tauri-serialized shape of `apps/pokerai-ui/src-tauri/src/error.rs`'s `AppError`
 *  (`#[serde(tag = "type", content = "detail")]`): `{type:"Engine",detail:{message}}`,
 *  `{type:"Busy"}`, `{type:"Closed"}`, `{type:"Serialization",detail:{message}}`,
 *  `{type:"Unsupported",detail:{reason}}`. */
function isRecord(x:unknown):x is Record<string,unknown>{return typeof x==='object'&&x!==null;}
interface AppErrorLike { type:string; detail?:unknown }
function isAppErrorLike(x:unknown):x is AppErrorLike { return isRecord(x)&&typeof x.type==='string'; }
function messageFromAppError(err:AppErrorLike):string {
  const d=err.detail;
  if((err.type==='Engine'||err.type==='Serialization')&&isRecord(d)&&typeof d.message==='string') return d.message;
  if(err.type==='Busy') return 'The engine is busy; try again.';
  if(err.type==='Closed') return 'The application is stopping.';
  if(err.type==='Unsupported') return 'This configuration is unsupported.';
  return `Configuration was rejected (${err.type}).`;
}
/** Best-effort field guess from a free-form Rust validation message, most specific keyword first;
 *  `null` (rendered as a form-level fallback, review R2) when nothing is identifiable. */
function fieldFromMessage(message:string):string|null {
  const m=message.toLowerCase();
  if(m.includes('straddle')) return 'straddle';
  if(m.includes('small blind')) return 'sb_chips';
  if(m.includes('big blind')) return 'bb_chips';
  if(m.includes('flop budget')||m.includes('flop_budget')) return 'flop_budget_s';
  if(m.includes('cap')) return 'cap';
  if(m.includes('rake')||m.includes('rate')||m.includes('percent')) return 'rate';
  if(m.includes('stack')) return 'stacks';
  if(m.includes('seat')||m.includes('dealt')) return 'seat';
  return null;
}
/** Normalizes any error the config form can encounter -- a local `FieldError`, a rejected Tauri
 *  command's serialized `AppError`, or an arbitrary `Error`/value -- into one field key (or `null`
 *  for a form-level fallback) and a display message (review R2). */
export function describeError(err:unknown):{field:string|null;message:string} {
  if(err instanceof FieldError) return {field:err.field,message:err.message};
  if(isAppErrorLike(err)){const message=messageFromAppError(err);return {field:fieldFromMessage(message),message};}
  if(err instanceof Error) return {field:fieldFromMessage(err.message),message:err.message};
  return {field:null,message:String(err)};
}
