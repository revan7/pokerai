import type {Action,HandState} from '../ipc/types.gen';
// Legality comes only from the engine's `Derived.legal` intervals: this module never derives a
// minimum raise, raise reopening, pot, stack or actor in JavaScript (spec §5.1: "Illegal keys are
// disabled from `Derived.legal`; the seat to act is always derived").
export function actionForKey(h:HandState|null,key:string):Action|null {
  if(!h||h.phase.phase!=='betting'||h.derived.to_act===null)return null;
  const legal=h.derived.legal;
  if(key==='F'&&legal.some(a=>a.kind==='fold'))return {kind:'fold'};
  if(key==='C'){
    if(legal.some(a=>a.kind==='check'))return {kind:'check'};
    if(legal.some(a=>a.kind==='call'))return {kind:'call'};
  }
  if(key==='A'){const allin=legal.find(a=>a.kind==='all_in');if(allin?.kind==='all_in')return {kind:'allin',to:allin.to};}
  return null;
}
export function canWager(h:HandState|null):boolean {
  return !!h&&h.phase.phase==='betting'&&h.derived.to_act!==null&&
    h.derived.legal.some(a=>a.kind==='bet'||a.kind==='raise');
}
// Bet/raise TO the typed amount. Out-of-interval amounts are rejected, never clamped.
export function wager(h:HandState,to:number):Action {
  if(!Number.isInteger(to)||to<0||to>4_294_967_295)throw new Error('Enter whole chips');
  const range=h.derived.legal.find(a=>(a.kind==='bet'||a.kind==='raise')&&to>=a.min_to&&to<=a.max_to);
  if(!range||(range.kind!=='bet'&&range.kind!=='raise'))throw new Error('Amount is outside the legal bet/raise-to interval');
  const allin=h.derived.legal.find(a=>a.kind==='all_in'&&a.to===to);
  return allin?.kind==='all_in'?{kind:'allin',to}:{kind:range.kind,to};
}
// The frozen keyboard vocabulary (spec §5.1): action letters (F,C,A,B,H,T,E,X,N), card rank/suit
// letters used by hero/board entry (ranks 2-9,T,J,Q,K,A; suits c,d,h,s), digits for wizard stacks
// and bet amounts, space (recommend), and the named/control keys. A key outside this set can never
// mean anything against any reachable snapshot -- not now, and not after further mutations change
// the mode -- so it is excluded before queue admission (`EntryController.key`) and before the
// native listener's `preventDefault` (`attachKeys`), rather than merely becoming a no-op once
// already admitted and holding a queue slot (I2). Letters not in this union (g,i,l,m,o,p,r,u,v,w,y,z)
// are never meaningful in any mode.
const MAPPED_SINGLE_CHAR=/^[a-fhjknqstx0-9 ]$/i;
const MAPPED_NAMED_KEYS=new Set(['Enter','Backspace','Escape','Ctrl+Z']);
export function isMappedKey(key:string):boolean {
  return MAPPED_SINGLE_CHAR.test(key)||MAPPED_NAMED_KEYS.has(key);
}
