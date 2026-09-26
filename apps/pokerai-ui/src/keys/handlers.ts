import type {Backend} from '../ipc/backend';
import type {GameConfig,HandState,Seat} from '../ipc/types.gen';
import type {StackDrafts} from '../state/session';
import {describeError} from '../state/session';
import {Recommendations} from '../state/events';
import {decisionReason,newWizard,wizardKey,wizardResult,prefill,parseCards} from '../state/hand';
import type {Wizard} from '../state/hand';
import {actionForKey,canWager,wager} from './keymap';
export type EntryState={hand:HandState|null;lastHand:HandState|null;wizard:Wizard|null;
  mode:'idle'|'hero'|'bet'|'board'|'tag';text:string;tagSeat:Seat|null;error:string|null;busy:boolean};
// Spec §5.1's key map, one entry point for keyboard and on-screen controls alike. Every hand
// mutation goes through an engine command (`begin_hand`, `set_hero_cards`, `apply_action`,
// `set_board`, `undo`, `finish_hand`, `abandon_hand`); the controller never edits `HandState`
// itself and keeps no history stack of its own.
export class EntryController {
  private value:EntryState={hand:null,lastHand:null,wizard:null,mode:'idle',text:'',tagSeat:null,error:null,busy:false};
  private listeners=new Set<()=>void>();private chain=Promise.resolve();private queued=0;private disposed=false;
  constructor(private backend:Backend,private rec:Recommendations,private config:GameConfig,private stacks:StackDrafts){}
  snapshot=()=>this.value;
  subscribe=(fn:()=>void)=>{this.listeners.add(fn);return()=>{this.listeners.delete(fn);};};
  private update(patch:Partial<EntryState>){this.value={...this.value,...patch};for(const f of this.listeners)f();}
  setSession(config:GameConfig,stacks:StackDrafts){this.config=config;this.stacks=stacks;}
  setHand(hand:HandState){this.update({hand,lastHand:hand});}
  // A rejected Tauri command arrives as the serialized `AppError` (`{type,detail:{message}}`), not
  // an `Error`; `describeError` (P5.T8) is the one normalizer that surfaces the engine's own message.
  report=(error:unknown)=>{this.update({error:describeError(error).message});};
  // Keys are handled strictly in order, each against the engine snapshot the previous key left
  // behind. At most 64 keys wait behind a pending IPC call; beyond that the key is dropped and the
  // overflow is shown instead of retaining unbounded input. A rejection never breaks the chain.
  key(key:string):Promise<void>{
    if(this.disposed)return Promise.resolve();
    if(this.queued>=64){this.report(new Error('Input queue full; wait for the current action'));return Promise.resolve();}
    this.queued++;
    this.chain=this.chain.then(()=>this.run(key)).catch(this.report).finally(()=>{this.queued--;});
    return this.chain;
  }
  // A rejected key press -- local validation or an engine rejection -- leaves the entry exactly as
  // it was before that key (open field, typed text, wizard step; the engine snapshot is only ever
  // replaced on success) and adds the error beside the field. Restoring the wizard matters: a
  // wizard left at `done` after a rejected `begin_hand` would re-send it on any later key.
  private async run(key:string):Promise<void>{
    const before=this.value;
    try{await this.handle(key);}
    catch(error){
      if(this.disposed)return;
      this.update({wizard:before.wizard,mode:before.mode,text:before.text,tagSeat:before.tagSeat,
        busy:false,error:describeError(error).message});
    }
  }
  // All mutations first invalidate the recommendation identity (spec §5 step 3); only a success
  // replaces the snapshot.
  private async mutate(run:()=>Promise<HandState>){
    this.rec.invalidate();this.update({busy:true,error:null});
    try{const hand=await run();if(this.disposed)return;
      this.update({hand,lastHand:hand,mode:'idle',text:'',wizard:null});
      if(decisionReason(hand)===null)void this.rec.request(hand);
    }finally{this.update({busy:false});}
  }
  private async handle(key:string):Promise<void>{
    if(this.disposed)return;
    const h=this.value.hand;
    if(key==='Ctrl+Z'){
      // Same gate as the on-screen Undo control: with neither a hand nor a previous one it is disabled.
      if(!h&&!this.value.lastHand)return;
      this.update({wizard:null,mode:'idle',text:''});await this.mutate(()=>this.backend.undo());return;
    }
    if(key==='Escape'){this.update({wizard:null,mode:'idle',text:'',tagSeat:null,error:null});return;}
    if(this.value.wizard){
      const wizard=wizardKey(this.value.wizard,key);this.update({wizard});const begin=wizardResult(wizard);
      if(begin)await this.mutate(()=>this.backend.begin_hand(begin));return;
    }
    if(this.value.mode==='bet'){
      if(key==='Enter'&&h){if(!/^\d+$/.test(this.value.text))throw new Error('Enter whole chips');const action=wager(h,Number(this.value.text));await this.mutate(()=>this.backend.apply_action(action));}
      else if(/^\d$/.test(key))this.update({text:this.value.text+key});
      else if(key==='Backspace')this.update({text:this.value.text.slice(0,-1)});return;
    }
    const upper=key.toUpperCase();
    // E/X terminate even a partial board entry; H opens hero entry when the board buffer is empty.
    if(h&&(upper==='E'||upper==='X')){
      this.rec.invalidate();this.update({busy:true});
      try{if(upper==='E')await this.backend.finish_hand();else await this.backend.abandon_hand();
        this.update({hand:null,lastHand:h,mode:'idle',text:'',error:null});
      }finally{this.update({busy:false});}return;
    }
    if(h&&upper==='H'&&this.value.mode!=='hero'&&this.value.text===''){
      this.update({mode:'hero',text:'',error:null});return;
    }
    // Board characters are accepted only in AwaitingBoard (never after Complete{AllInRunout}); there
    // T is the ten rank, and F/C/A are not actions.
    const awaiting=!!h&&h.phase.phase==='awaiting_board';
    if(this.value.mode==='hero'||awaiting){
      if(key==='Backspace'){this.update({text:this.value.text.slice(0,-1)});return;}
      const hero=this.value.mode==='hero';
      if(key!=='Enter'&&/^[2-9tjqkacdhs]$/i.test(key))this.update({text:this.value.text+key,mode:hero?'hero':'board'});
      if(h&&((hero&&this.value.text.length===4)||(!hero&&key==='Enter'))){
        const cards=parseCards(this.value.text);
        if(hero){
          const a=cards[0],b=cards[1];if(cards.length!==2||a===undefined||b===undefined)throw new Error('Enter two hero cards');
          if(cards.some(c=>h.board.includes(c)))throw new Error('Hero cards duplicate board');
          await this.mutate(()=>this.backend.set_hero_cards([a,b]));
        }else{
          const count=h.board.length===0?3:1;if(cards.length!==count)throw new Error(`Enter ${count} board card(s)`);
          const board=[...h.board,...cards];if(new Set(board).size!==board.length||cards.some(c=>h.hero_cards?.includes(c)))throw new Error('Duplicate board or hero card');
          await this.mutate(()=>this.backend.set_board(board));
        }
      }return;
    }
    if(this.value.mode==='tag')return;
    switch(upper){
      case 'N':{
        if(h&&(h.phase.phase==='betting'||h.phase.phase==='awaiting_board'))throw new Error('Finish (E) or abandon (X) before a new hand');
        // MA-3: `.map(s=>[a,b])` infers `number[][]`, missing the tuple overload of
        // `Object.fromEntries` and falling through to its `any`-returning signature. `as const`
        // on each pair keeps the tuple type, and the explicit annotation catches any further drift.
        const defaults:StackDrafts={...Object.fromEntries(
          this.config.seats.map(s=>[s.seat,100*this.config.bb_chips] as const)),...this.stacks};
        this.update({wizard:newWizard(this.config.seats.map(s=>s.seat),prefill(this.value.lastHand,defaults)),error:null});return;
      }
      // MI-11: no `case 'H':` here — the explicit `if(h&&upper==='H'&&...)` branch above already
      // returns for every 'H' that could reach this switch (mode/text conditions are otherwise
      // guaranteed by the branches above it), so a second case is unreachable dead code.
      case 'B':if(canWager(h))this.update({mode:'bet',text:'',error:null});return;
      case ' ':if(h&&decisionReason(h)===null)void this.rec.request(h);return;
      case 'T':if(h&&h.derived.to_act!==null)this.update({mode:'tag',tagSeat:h.derived.to_act});return;
    }
    // F/C/A always act for `Derived.to_act`; any other key reaches here and is a no-op.
    const action=actionForKey(h,upper);if(action)await this.mutate(()=>this.backend.apply_action(action));
  }
  dispose(){this.disposed=true;this.rec.dispose();this.listeners.clear();}
}
// Window-level keyboard entry. Key repeats (a held key), IME composition, Alt/Meta chords and any
// focused native form control are never intercepted; only the mapped characters and named keys are
// forwarded (and have their browser default prevented).
export function attachKeys(target:Window,controller:EntryController):()=>void{
  const handler=(event:KeyboardEvent)=>{
    if(event.repeat||event.isComposing||event.altKey||event.metaKey)return;
    const element=event.target;
    if(element instanceof HTMLElement&&(element.matches('input,textarea,select')||element.isContentEditable))return;
    if(event.ctrlKey&&event.key.toLowerCase()!=='z')return;
    const key=event.ctrlKey?'Ctrl+Z':event.key;
    if(!/^[a-z0-9 ]$/i.test(key)&&!['Enter','Backspace','Escape','Ctrl+Z'].includes(key))return;
    event.preventDefault();void controller.key(key);
  };
  target.addEventListener('keydown',handler);return()=>target.removeEventListener('keydown',handler);
}
