import {expect,test} from 'vitest';
import {render,screen,waitFor} from '@testing-library/react';
import {newWizard,wizardKey,wizardResult,prefill,parseCards,cardText,decisionReason} from '../state/hand';
import {hand,config,identity} from './fixtures';
import {FakeBackend} from './fakeBackend';
import {installMockIpc} from './mockIpc';
import {tauriBackend} from '../ipc/backend';
import type {HandState} from '../ipc/types.gen';
import {Recommendations} from '../state/events';
import {EntryController,attachKeys} from '../keys/handlers';
import {Entry} from '../components/Entry';
import {Table} from '../components/Table';

test('wizard_defers_begin_until_every_stack_is_confirmed',()=>{
  const previous=hand();const stacks=prefill(previous,{});
  expect(stacks[1]).toBe(995);expect(stacks[2]).toBe(990);
  let w=newWizard([0,1,2,3,4,5],stacks);
  for(const key of ['1','1','Enter'])w=wizardKey(w,key); // button seat 0, hero seat 0, dealt confirmed
  for(let i=0;i<5;i++){
    w=wizardKey(w,'Enter'); // confirm one more of six stacks
    expect(wizardResult(w)).toBeNull(); // still incomplete after every partial confirmation
  }
  w=wizardKey(w,'Enter'); // sixth stack confirmed; now the cards prompt
  expect(w.step).toBe('cards');
  w=wizardKey(w,'Enter'); // defer hero cards to H
  expect(wizardResult(w)).toEqual({button:0,hero:0,dealt:[0,1,2,3,4,5],
    stacks:[1000,995,990,1000,1000,1000],hero_cards:null});
});

test('cards_and_derived_decision_gate',()=>{
  expect(parseCards('AsKd')).toEqual(['As','Kd']);expect(cardText('As')).toBe('As');
  expect(()=>parseCards('AsAs')).toThrow('Duplicate card'); // duplicate within the hero's own cards
  expect(()=>parseCards('1x')).toThrow('Cards use ranks 2–9,T,J,Q,K,A and suits c,d,h,s'); // malformed rank/suit
  // board duplication with a hero card: a combined validation call (hero cards followed by a
  // board card) rejects the same duplicate check parseCards uses for the hero's own pair
  expect(()=>parseCards('AsKdAs')).toThrow('Duplicate card');
  const h=hand();expect(decisionReason(h)).toBe('another seat to act');
  expect(decisionReason({...h,hero_cards:null})).toBe('hero cards unknown');
  const heroTurn={...h,derived:{...h.derived,to_act:0}};
  expect(decisionReason(heroTurn)).toBeNull();
  expect(decisionReason({...heroTurn,derived:{...heroTurn.derived,legal:[{kind:'check'}]}})).toBe('fewer than two legal actions');
});

test('decisionReason_covers_every_named_reason',()=>{
  expect(decisionReason(null)).toBe('no hand');
  // all-in survivor case: hand ended via an all-in runout, but hero personally still has chips
  // (never went all-in) -- the phase gate must still block any decision, not the all_in check
  const survivor=hand({phase:{phase:'complete',reason:'all_in_runout'}});
  expect(survivor.derived.all_in[survivor.hero]).toBe(false);
  expect(decisionReason(survivor)).toBe('hand complete or awaiting board');
  const awaitingBoard=hand({phase:{phase:'awaiting_board',street:'flop'}});
  expect(decisionReason(awaitingBoard)).toBe('hand complete or awaiting board');
  const abandoned=hand({phase:{phase:'abandoned'}});
  expect(decisionReason(abandoned)).toBe('hand complete or awaiting board');
  const notDealt=hand({hero:5,dealt:[0,1,2,3,4]});
  expect(decisionReason(notDealt)).toBe('hero is not dealt');
  const folded=hand();folded.derived.folded[0]=true;
  expect(decisionReason(folded)).toBe('hero folded');
  const allIn=hand();allIn.derived.all_in[0]=true;
  expect(decisionReason(allIn)).toBe('hero all-in');
  const noCards=hand({hero_cards:null});
  expect(decisionReason(noCards)).toBe('hero cards unknown');
  const anotherToAct=hand(); // default fixture already has to_act:3, hero:0
  expect(decisionReason(anotherToAct)).toBe('another seat to act');
  const heroTurn=hand();heroTurn.derived.to_act=0;
  expect(decisionReason(heroTurn)).toBeNull();
  const fewLegal=hand();fewLegal.derived.to_act=0;fewLegal.derived.legal=[{kind:'check'}];
  expect(decisionReason(fewLegal)).toBe('fewer than two legal actions');
});

test('sparse_seats_prefill_uses_dealt_starts_and_physical_remaining',()=>{
  const h=hand({dealt:[0,2,5],stacks_start:[1000,800,1200]});
  h.derived.stacks_remaining=[975,0,750,0,0,1160];
  expect(prefill(h,{})).toEqual({0:975,2:750,5:1160});
  let w=newWizard([0,2,5],{0:975,2:750,5:1160});
  for(const key of ['1','1','Enter','Enter','Enter','Enter','Enter'])w=wizardKey(w,key);
  expect(wizardResult(w)?.stacks).toEqual([975,750,1160]);
});

test('three_seat_dealt_vector_confirms_each_stack_before_result_is_available',()=>{
  const h=hand({dealt:[0,2,5],stacks_start:[1000,800,1200]});
  h.derived.stacks_remaining=[975,0,750,0,0,1160];
  const stacks=prefill(h,{});
  expect(stacks).toEqual({0:975,2:750,5:1160});
  let w=newWizard([0,2,5],stacks);
  w=wizardKey(w,'1');w=wizardKey(w,'1');w=wizardKey(w,'Enter'); // button 0, hero 0, dealt vector confirmed
  expect(w.step).toBe('stack');expect(w.confirmed).toEqual([]);
  w=wizardKey(w,'Enter'); // confirm seat 0's stack (975)
  expect(w.confirmed).toEqual([0]);expect(wizardResult(w)).toBeNull();
  w=wizardKey(w,'Enter'); // confirm seat 2's stack (750)
  expect(w.confirmed).toEqual([0,2]);expect(wizardResult(w)).toBeNull();
  w=wizardKey(w,'Enter'); // confirm seat 5's stack (1160); all three confirmed, now the cards prompt
  expect(w.confirmed).toEqual([0,2,5]);expect(w.step).toBe('cards');expect(wizardResult(w)).toBeNull();
  w=wizardKey(w,'Enter'); // defer hero cards
  expect(wizardResult(w)).toEqual({button:0,hero:0,dealt:[0,2,5],stacks:[975,750,1160],hero_cards:null});
});

test('zero_prefill_from_completed_and_abandoned_snapshots_requires_explicit_confirmation',()=>{
  const completed=hand({dealt:[0,1,2],stacks_start:[1000,1000,1000],
    phase:{phase:'complete',reason:'all_in_runout'}});
  completed.derived.stacks_remaining=[0,2000,1000,0,0,0]; // seat 0 busted
  const stacksFromCompleted=prefill(completed,{});
  expect(stacksFromCompleted).toEqual({0:0,1:2000,2:1000});

  const abandoned=hand({dealt:[0,1,2],stacks_start:[1000,1000,1000],phase:{phase:'abandoned'}});
  abandoned.derived.stacks_remaining=[0,2000,1000,0,0,0];
  const stacksFromAbandoned=prefill(abandoned,{});
  expect(stacksFromAbandoned).toEqual({0:0,1:2000,2:1000}); // an ended-by-E/X hand still supplies its last snapshot

  let w=newWizard([0,1,2],stacksFromCompleted);
  w=wizardKey(w,'1');w=wizardKey(w,'1');w=wizardKey(w,'Enter'); // button 0, hero 0, dealt confirmed
  expect(w.step).toBe('stack');expect(w.text).toBe('0');expect(w.confirmed).toEqual([]);
  const beforeConfirm=w;
  expect(wizardKey(w,'x')).toBe(beforeConfirm); // an irrelevant key does not silently confirm the zero stack
  w=wizardKey(w,'Enter'); // the zero stack still requires its own explicit Enter
  expect(w.confirmed).toEqual([0]);expect(w.stacks[0]).toBe(0);
  expect(wizardResult(w)).toBeNull();
});

test('stack_step_editing_backspace_and_invalid_submission_recovery',()=>{
  let w=newWizard([0,1,2],{0:1000,1:1000,2:1000});
  for(const key of ['1','1','Enter'])w=wizardKey(w,key); // button 0, hero 0, dealt confirmed
  expect(w.step).toBe('stack');expect(w.text).toBe('1000');expect(w.pristine).toBe(true);
  w=wizardKey(w,'5'); // typing over a pristine prefilled value replaces it
  expect(w.text).toBe('5');expect(w.pristine).toBe(false);
  w=wizardKey(w,'Backspace');expect(w.text).toBe('');
  for(const key of ['2','0','0'])w=wizardKey(w,key);
  expect(w.text).toBe('200');
  const beforeIgnored=w;
  expect(wizardKey(w,'!')).toBe(beforeIgnored); // an irrelevant key is a no-op
  expect(()=>wizardKey({...w,text:'20x'},'Enter')).toThrow('Stack must be whole chips'); // malformed
  expect(()=>wizardKey({...w,text:'9999999999'},'Enter')).toThrow('Stack must be whole chips'); // overflow
  expect(w.confirmed).toEqual([]); // neither failed attempt touched this still-unconfirmed wizard
  w=wizardKey(w,'Enter'); // the untouched, still-valid '200' now submits
  expect(w.confirmed).toEqual([0]);expect(w.stacks[0]).toBe(200);
  expect(wizardResult(w)).toBeNull(); // two more seats remain
});

test('keys_after_done_are_no_ops',()=>{
  let w=newWizard([0,1,2],{0:1000,1:1000,2:1000});
  for(const key of ['1','1','Enter','Enter','Enter','Enter'])w=wizardKey(w,key); // through the cards step
  expect(w.step).toBe('cards');
  w=wizardKey(w,'Enter'); // defer hero cards -> done
  expect(w.step).toBe('done');
  const result=wizardResult(w);
  expect(result).not.toBeNull();
  expect(wizardKey(w,'Enter')).toBe(w); // identical reference: wizardKey short-circuits once done
  expect(wizardKey(w,'3')).toBe(w);
  expect(wizardKey(w,'Backspace')).toBe(w);
  expect(wizardResult(wizardKey(w,'Enter'))).toEqual(result);
});

test('undo_is_engine_command',async()=>{
  const fake=new FakeBackend();installMockIpc(fake);
  const h=hand();fake.hands=[hand({hand_revision:42})];
  const rec=new Recommendations(tauriBackend,()=>{});
  const entry=new EntryController(tauriBackend,rec,config,{});entry.setHand(h);
  await entry.key('Ctrl+Z');
  expect(fake.calls.filter(([name])=>name==='undo')).toHaveLength(1);
  expect(entry.snapshot().hand?.hand_revision).toBe(42);
  expect(entry.snapshot().hand).toEqual(hand({hand_revision:42}));entry.dispose();
});
test('illegal_keys_disabled',async()=>{
  const fake=new FakeBackend();installMockIpc(fake);
  const rec=new Recommendations(tauriBackend,()=>{});
  const entry=new EntryController(tauriBackend,rec,config,{});
  const h=hand();entry.setHand({...h,derived:{...h.derived,legal:[{kind:'check'}]}});
  render(<Entry state={entry.snapshot()} press={key=>entry.key(key)}/>);
  expect(screen.getByRole('button',{name:'F Fold'})).toBeDisabled();
  expect(screen.getByRole('button',{name:'A All-in'})).toBeDisabled();
  expect(screen.getByRole('button',{name:'B Bet/raise to'})).toBeDisabled();
  for(const key of ['F','A','B'])await entry.key(key);
  expect(fake.calls).toHaveLength(0);entry.dispose();
});
test('begin_hand_stack_confirmation',async()=>{
  // MA-9: spec §13.4 names this test; it must exercise the *behavioural* requirement spec §4.3
  // actually cares about — no `begin_hand` command reaches the backend before every selected stack
  // is confirmed — by driving the real controller. Task 9's `wizard_defers_...` test covers the
  // pure `wizardKey`/`wizardResult` reducer directly but, on its own, could never fail this
  // requirement, since it never constructs an `EntryController` or inspects `fake.calls`.
  const fake=new FakeBackend();installMockIpc(fake);fake.hands=[hand()];
  const rec=new Recommendations(tauriBackend,()=>{});
  const entry=new EntryController(tauriBackend,rec,config,{});
  await entry.key('N');await entry.key('1');await entry.key('1');await entry.key('Enter'); // button, hero, dealt
  for(let i=0;i<5;i++){
    await entry.key('Enter'); // confirm one more of six stacks
    expect(fake.calls.filter(([name])=>name==='begin_hand')).toHaveLength(0);
  }
  await entry.key('Enter'); // sixth stack confirmed; wizard now prompts for cards
  await entry.key('Enter'); // defer hero cards to H
  expect(fake.calls.filter(([name])=>name==='begin_hand')).toHaveLength(1);
  entry.dispose();
});
test('bet_bounds_escape_call_allin_and_undo_clear_partial_input',async()=>{
  const fake=new FakeBackend();installMockIpc(fake);
  const rec=new Recommendations(tauriBackend,()=>{});
  const entry=new EntryController(tauriBackend,rec,config,{});
  entry.setHand(hand()); // legal: fold, call(10), raise(20..1000), all_in(1000)
  await entry.key('B');await entry.key('1');await entry.key('9');await entry.key('Enter'); // 19 < min_to 20
  expect(entry.snapshot().error).toMatch(/legal bet\/raise-to interval/);
  expect(fake.calls).toHaveLength(0);
  await entry.key('Escape');
  await entry.key('B');await entry.key('9');await entry.key('9');await entry.key('9');await entry.key('9');await entry.key('Enter'); // 9999 > max_to 1000
  expect(entry.snapshot().error).toMatch(/legal bet\/raise-to interval/);
  expect(fake.calls).toHaveLength(0);
  await entry.key('Escape');
  await entry.key('B');await entry.key('5');await entry.key('0');await entry.key('Escape');
  expect(entry.snapshot().mode).toBe('idle');expect(entry.snapshot().text).toBe('');
  expect(fake.calls).toHaveLength(0);
  const checkable={...hand(),derived:{...hand().derived,legal:[{kind:'check'} as const,{kind:'call',cost:10} as const]}};
  entry.setHand(checkable);fake.hands=[hand({hand_revision:2})];
  await entry.key('C'); // C chooses Check before Call when both are legal.
  expect(fake.calls.filter(([n])=>n==='apply_action').at(-1)).toEqual(['apply_action',{action:{kind:'check'}}]);
  entry.setHand(hand());fake.hands=[hand({hand_revision:3})];
  await entry.key('A'); // A uses the engine's own `to`, never a locally recomputed stack.
  expect(fake.calls.filter(([n])=>n==='apply_action').at(-1)).toEqual(['apply_action',{action:{kind:'allin',to:1000}}]);
  fake.hands=[hand({hand_revision:4})];
  await entry.key('B');await entry.key('7');await entry.key('Ctrl+Z'); // clears partial bet text too.
  expect(entry.snapshot().mode).toBe('idle');expect(entry.snapshot().text).toBe('');
  expect(fake.calls.filter(([n])=>n==='undo')).toHaveLength(1);
  entry.dispose();
});
test('attach_keys_ignores_repeats_and_native_form_inputs',()=>{
  const calls:string[]=[];
  const stub={key:(k:string)=>{calls.push(k);return Promise.resolve();}} as unknown as EntryController;
  const input=document.createElement('input');document.body.appendChild(input);
  const remove=attachKeys(window,stub);
  input.dispatchEvent(new KeyboardEvent('keydown',{key:'f',bubbles:true,cancelable:true}));
  expect(calls).toHaveLength(0); // a focused native <input> is never intercepted
  window.dispatchEvent(new KeyboardEvent('keydown',{key:'f',repeat:true,cancelable:true}));
  expect(calls).toHaveLength(0); // a held key's repeat events never reach the controller
  window.dispatchEvent(new KeyboardEvent('keydown',{key:'f',cancelable:true}));
  expect(calls).toEqual(['f']);
  remove();document.body.removeChild(input);
});

// ---- Standing rulings for the key map (orchestrator, P5.T10 dispatch) ----
// One controller per scenario, wired through the real `tauriBackend` + mockIPC seam so every
// command's argument shape is validated exactly as a Tauri `invoke` would carry it.
function controller(h:HandState|null=hand()){
  const fake=new FakeBackend();installMockIpc(fake);
  const rec=new Recommendations(tauriBackend,()=>{});
  const entry=new EntryController(tauriBackend,rec,config,{});
  if(h)entry.setHand(h);
  return {fake,rec,entry};
}
// Mirrors the Tauri-serialized `AppError` (`{type,detail}`, src-tauri/src/error.rs) as a real
// `Error` subclass, the same way session.test.tsx scripts an engine rejection.
class FakeAppError extends Error {
  readonly type:string;
  readonly detail:unknown;
  constructor(type:string,detail:unknown){super(`fake AppError: ${type}`);this.name='FakeAppError';this.type=type;this.detail=detail;}
}

test('each_mapped_key_does_exactly_one_thing',async()=>{
  // Letters arrive lowercase from `attachKeys` (event.key without Shift). F/C/A act for
  // Derived.to_act (seat 4 in the fixture, a villain), each as exactly one engine command.
  for(const [key,action] of [['f',{kind:'fold'}],['c',{kind:'call'}],['a',{kind:'allin',to:1000}]] as const){
    const {fake,entry}=controller();fake.hands=[hand({hand_revision:8})];
    await entry.key(key);
    expect(fake.calls).toEqual([['apply_action',{action}]]);
    expect(entry.snapshot().hand?.hand_revision).toBe(8);entry.dispose();
  }
  // B, H and T only open their own entry field; nothing reaches the engine.
  for(const [key,mode] of [['b','bet'],['h','hero'],['t','tag']] as const){
    const {fake,entry}=controller();await entry.key(key);
    expect(entry.snapshot().mode).toBe(mode);expect(fake.calls).toHaveLength(0);entry.dispose();
  }
  {const {entry}=controller();await entry.key('t');expect(entry.snapshot().tagSeat).toBe(3);entry.dispose();}
  // Space re-requests exactly one recommendation, and only when hero is the one deciding.
  {const heroTurn=hand();heroTurn.derived.to_act=0;const {fake,entry}=controller(heroTurn);
    await entry.key(' ');expect(fake.calls.map(([n])=>n)).toEqual(['recommend']);entry.dispose();}
  // E / X end the hand with exactly one command; the ended snapshot becomes the prefill source.
  for(const [key,command] of [['e','finish_hand'],['x','abandon_hand']] as const){
    const {fake,entry}=controller();await entry.key(key);
    expect(fake.calls).toEqual([[command,{}]]);
    expect(entry.snapshot().hand).toBeNull();expect(entry.snapshot().lastHand).toEqual(hand());entry.dispose();
  }
  // Ctrl+Z is exactly one engine `undo`.
  {const {fake,entry}=controller();fake.hands=[hand({hand_revision:6})];await entry.key('Ctrl+Z');
    expect(fake.calls).toEqual([['undo',{}]]);expect(entry.snapshot().hand?.hand_revision).toBe(6);entry.dispose();}
  // N opens the wizard without a command, and is refused while a hand is live.
  {const {fake,entry}=controller(null);await entry.key('n');
    expect(entry.snapshot().wizard?.step).toBe('button');expect(fake.calls).toHaveLength(0);entry.dispose();}
  {const {fake,entry}=controller();await entry.key('n');expect(entry.snapshot().wizard).toBeNull();
    expect(entry.snapshot().error).toMatch(/Finish \(E\) or abandon \(X\)/);expect(fake.calls).toHaveLength(0);entry.dispose();}
  // Escape closes the open field and nothing else.
  {const {fake,entry}=controller();await entry.key('b');await entry.key('4');await entry.key('Escape');
    expect(entry.snapshot()).toMatchObject({mode:'idle',text:'',hand:hand()});expect(fake.calls).toHaveLength(0);entry.dispose();}
});

test('unmapped_keys_are_no_ops',async()=>{
  const {fake,entry}=controller();const before=entry.snapshot();
  for(const key of ['q','z','g','0','1','9','Enter','Backspace','Tab','ArrowUp','Delete'])await entry.key(key);
  expect(fake.calls).toHaveLength(0);expect(entry.snapshot()).toBe(before); // not even a re-render
  // With neither a hand nor a previous one the Undo control is disabled, and so is its key.
  const idle=controller(null);await idle.entry.key('Ctrl+Z');
  expect(idle.fake.calls).toHaveLength(0);expect(idle.entry.snapshot().error).toBeNull();
  // The window listener neither forwards nor swallows keys outside the map.
  const calls:string[]=[];
  const stub={key:(k:string)=>{calls.push(k);return Promise.resolve();}} as unknown as EntryController;
  const remove=attachKeys(window,stub);
  for(const init of [{key:'Tab'},{key:'ArrowUp'},{key:'c',ctrlKey:true},{key:'f',altKey:true},{key:'Shift',shiftKey:true},
    {key:'Z',ctrlKey:true,shiftKey:true}]){ // I1: an extra modifier on the Ctrl+Z chord is unmapped, not Undo
    const event=new KeyboardEvent('keydown',{...init,cancelable:true});window.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
  }
  expect(calls).toHaveLength(0);
  const mapped=new KeyboardEvent('keydown',{key:'z',ctrlKey:true,cancelable:true});window.dispatchEvent(mapped);
  expect(mapped.defaultPrevented).toBe(true);expect(calls).toEqual(['Ctrl+Z']);
  // Ordinary shifted letters for uppercase action/card entry keep working (no ctrlKey involved).
  const shiftedLetter=new KeyboardEvent('keydown',{key:'F',shiftKey:true,cancelable:true});window.dispatchEvent(shiftedLetter);
  expect(shiftedLetter.defaultPrevented).toBe(true);expect(calls).toEqual(['Ctrl+Z','F']);
  remove();entry.dispose();idle.entry.dispose();
});

test('engine_rejection_is_a_field_error_and_leaves_state_unchanged',async()=>{
  const {fake,entry}=controller();const h=entry.snapshot().hand;
  const scripted=fake.apply_action.bind(fake);
  fake.apply_action=action=>{fake.apply_action=scripted;fake.calls.push(['apply_action',{action}]);
    return Promise.reject(new FakeAppError('Engine',{message:'raise to 30 does not reopen the betting'}));};
  for(const key of ['b','3','0','Enter'])await entry.key(key);
  expect(fake.calls).toEqual([['apply_action',{action:{kind:'raise',to:30}}]]);
  expect(entry.snapshot()).toMatchObject({mode:'bet',text:'30',busy:false,error:'raise to 30 does not reopen the betting'});
  expect(entry.snapshot().hand).toBe(h); // the last engine snapshot, untouched
  render(<Entry state={entry.snapshot()} press={key=>entry.key(key)}/>);
  expect(screen.getByRole('alert')).toHaveTextContent('raise to 30 does not reopen the betting');
  expect(screen.getByTestId('entry-prompt')).toHaveTextContent('Bet/raise TO (chips): 30');
  expect(screen.getByTestId('entry-prompt')).toHaveAccessibleDescription('raise to 30 does not reopen the betting');
  // The rejection never breaks later input: the same Enter retries against the same snapshot.
  fake.hands=[hand({hand_revision:8})];await entry.key('Enter');
  expect(fake.calls).toHaveLength(2);
  expect(entry.snapshot()).toMatchObject({mode:'idle',text:'',error:null});
  expect(entry.snapshot().hand?.hand_revision).toBe(8);entry.dispose();
  // A rejected begin_hand leaves the wizard where it was, so no later key silently re-sends it.
  const next=controller(null);const begin=next.fake.begin_hand.bind(next.fake);
  next.fake.begin_hand=b=>{next.fake.begin_hand=begin;next.fake.calls.push(['begin_hand',{begin:b}]);
    return Promise.reject(new FakeAppError('Engine',{message:'stacks exceed the chip limit'}));};
  for(const key of ['n','1','1','Enter','Enter','Enter','Enter','Enter','Enter','Enter','Enter'])await next.entry.key(key);
  const begins=()=>next.fake.calls.filter(([n])=>n==='begin_hand');
  expect(begins()).toHaveLength(1);
  expect(next.entry.snapshot().error).toBe('stacks exceed the chip limit');
  expect(next.entry.snapshot().wizard?.step).toBe('cards');expect(next.entry.snapshot().hand).toBeNull();
  await next.entry.key('5');await next.entry.key('Backspace');
  expect(begins()).toHaveLength(1);
  next.fake.hands=[hand()];await next.entry.key('Enter');
  expect(begins()).toHaveLength(2);
  expect(next.entry.snapshot().wizard).toBeNull();expect(next.entry.snapshot().hand).toEqual(hand());
  next.entry.dispose();
});

test('input_is_serialized_against_the_next_snapshot_and_capped_at_64_keys',async()=>{
  const {fake,entry}=controller();
  let release=():void=>undefined;const held=new Promise<void>(resolve=>{release=resolve;});
  const scripted=fake.apply_action.bind(fake);
  fake.apply_action=async action=>{const result=scripted(action);await held;return result;};
  const checkable=hand({hand_revision:8});
  checkable.derived.legal=[{kind:'check'},{kind:'bet',min_to:10,max_to:990},{kind:'all_in',to:990}];
  fake.hands=[checkable,hand({hand_revision:9})];
  const first=entry.key('f');
  // I2: a burst of globally unmapped letters (never meaningful in any reachable mode) must never
  // consume queue capacity or delay/displace a mapped key queued right after it.
  const unmappedBurst=Array.from({length:70},()=>entry.key('g'));
  const second=entry.key('c');
  // 62 mapped filler keys (digits: they can become meaningful, e.g. typed against a later bet
  // prompt) legitimately occupy the remaining queue capacity: first(1)+second(1)+fillers(62)=64.
  const fillers=Array.from({length:62},()=>entry.key('0'));
  expect(entry.snapshot().error).toBeNull(); // the 70-key unmapped burst never overflowed the queue
  const dropped=entry.key('f'); // the 65th *mapped* key while the first IPC call is still pending
  expect(entry.snapshot().error).toMatch(/Input queue full/);
  release();await Promise.all([first,...unmappedBurst,second,...fillers,dropped]);
  // `c` ran against the snapshot the fold returned (Check legal there), never the one it was typed
  // on; it was neither dropped nor reordered by the unmapped burst queued ahead of it.
  expect(fake.calls).toEqual([['apply_action',{action:{kind:'fold'}}],['apply_action',{action:{kind:'check'}}]]);
  expect(entry.snapshot().hand?.hand_revision).toBe(9);entry.dispose();
});

test('ctrl_z_is_gated_by_the_same_disabled_undo_predicate_as_the_button',async()=>{
  // I3: Ctrl+Z must be a no-op whenever the on-screen Undo control would be disabled, including
  // the `busy` condition (not just the no-hand/no-lastHand condition already covered elsewhere).
  const {fake,entry}=controller();
  let release=():void=>undefined;const held=new Promise<void>(resolve=>{release=resolve;});
  const scripted=fake.apply_action.bind(fake);
  fake.apply_action=async action=>{const result=scripted(action);await held;return result;};
  fake.hands=[hand({hand_revision:5})];
  const pending=entry.key('f'); // a genuinely pending mutation: the engine call is held open
  await waitFor(()=>expect(entry.snapshot().busy).toBe(true));
  render(<Entry state={entry.snapshot()} press={key=>entry.key(key)}/>);
  expect(screen.getByRole('button',{name:'Undo'})).toBeDisabled();
  const ctrlZ=entry.key('Ctrl+Z'); // pressed while busy, exactly when the Undo control is disabled
  release();await Promise.all([pending,ctrlZ]);
  expect(fake.calls).toEqual([['apply_action',{action:{kind:'fold'}}]]); // no undo was ever issued
  expect(entry.snapshot().busy).toBe(false);entry.dispose();
});

test('every_mutation_invalidates_the_recommendation_identity_first',async()=>{
  const heroTurn=hand();heroTurn.derived.to_act=0;
  const {fake,rec,entry}=controller(heroTurn);
  await entry.key(' ');await waitFor(()=>expect(rec.snapshot().active).toEqual(identity));
  fake.hands=[hand({hand_revision:8})];await entry.key('f');
  expect(fake.calls.map(([n])=>n)).toEqual(['recommend','cancel','apply_action']);
  expect(fake.calls[1]).toEqual(['cancel',{decision_id:90}]);expect(rec.snapshot().active).toBeNull();
  entry.setHand(heroTurn);await entry.key(' ');await waitFor(()=>expect(rec.snapshot().active).toEqual(identity));
  const mark=fake.calls.length;await entry.key('e');
  expect(fake.calls.slice(mark).map(([n])=>n)).toEqual(['cancel','finish_hand']);
  expect(rec.snapshot().active).toBeNull();entry.dispose();
});

test('hero_and_board_entry_reject_duplicates_and_stop_after_an_all_in_runout',async()=>{
  const flopBetting=hand({board:['Kh','7d','2c'],hero_cards:null,phase:{phase:'betting',street:'flop'}});
  const {fake,entry}=controller(flopBetting);
  for(const key of ['h','k','h','q','s'])await entry.key(key); // Kh is already on the board
  expect(entry.snapshot().error).toBe('Hero cards duplicate board');
  expect(entry.snapshot()).toMatchObject({mode:'hero',text:'khq'});expect(fake.calls).toHaveLength(0);
  for(const key of ['Backspace','Backspace','Backspace'])await entry.key(key);
  fake.hands=[hand({hand_revision:8})];
  for(const key of ['a','s','k','d'])await entry.key(key);
  expect(fake.calls).toEqual([['set_hero_cards',{cards:['As','Kd']}]]);entry.dispose();
  // AwaitingBoard: rank+suit characters then Enter; duplicates and wrong counts never reach the engine.
  const awaiting=hand({phase:{phase:'awaiting_board',street:'flop'}});awaiting.derived.to_act=null;awaiting.derived.legal=[];
  const board=controller(awaiting);
  for(const key of ['a','s','7','d','2','c','Enter'])await board.entry.key(key); // As is hero's card
  expect(board.entry.snapshot().error).toBe('Duplicate board or hero card');
  await board.entry.key('Escape');
  for(const key of ['k','h','7','d','Enter'])await board.entry.key(key);
  expect(board.entry.snapshot().error).toBe('Enter 3 board card(s)');expect(board.fake.calls).toHaveLength(0);
  board.fake.hands=[hand({hand_revision:8,board:['Kh','7d','2c'],phase:{phase:'betting',street:'flop'}})];
  for(const key of ['2','c','Enter'])await board.entry.key(key);
  expect(board.fake.calls).toEqual([['set_board',{board:['Kh','7d','2c']}]]);board.entry.dispose();
  // Ruling (h): after Complete{AllInRunout} no board entry is accepted or solicited.
  const runout=hand({phase:{phase:'complete',reason:'all_in_runout'}});runout.derived.to_act=null;runout.derived.legal=[];
  const done=controller(runout);
  for(const key of ['k','s','7','d','2','c','Enter'])await done.entry.key(key);
  expect(done.fake.calls).toHaveLength(0);expect(done.entry.snapshot()).toMatchObject({mode:'idle',text:''});
  render(<Entry state={done.entry.snapshot()} press={key=>done.entry.key(key)}/>);
  expect(screen.getByTestId('entry-prompt')).not.toHaveTextContent(/board/i);done.entry.dispose();
});

test('table_shows_the_engine_actor_and_controls_share_the_key_path',async()=>{
  const {fake,entry}=controller();fake.hands=[hand({hand_revision:8})];
  render(<><Table hand={entry.snapshot().hand}/><Entry state={entry.snapshot()} press={key=>entry.key(key)}/></>);
  expect(screen.getByTestId('seat-to-act')).toHaveTextContent('Seat 4 to act');
  expect(screen.getByTestId('table')).toHaveAttribute('data-hand-revision','7');
  expect(screen.getByRole('row',{current:true})).toHaveTextContent('Seat 4');
  screen.getByRole('button',{name:'C Check/call'}).click(); // the button runs the identical `key('C')` path
  await waitFor(()=>expect(fake.calls).toEqual([['apply_action',{action:{kind:'call'}}]]));
  await waitFor(()=>expect(entry.snapshot().hand?.hand_revision).toBe(8));entry.dispose();
  const runout=hand({phase:{phase:'complete',reason:'all_in_runout'}});runout.derived.to_act=null;
  render(<Table hand={runout}/>);expect(screen.getAllByTestId('seat-to-act').at(-1)).toHaveTextContent('No seat to act');
  render(<Table hand={null}/>);expect(screen.getByText('No hand. Press N.')).toBeVisible();
});
