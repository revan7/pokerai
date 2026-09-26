import {expect,test} from 'vitest';
import {newWizard,wizardKey,wizardResult,prefill,parseCards,cardText,decisionReason} from '../state/hand';
import {hand} from './fixtures';

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
