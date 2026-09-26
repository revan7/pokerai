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
  expect(()=>parseCards('AsAs')).toThrow('Duplicate');expect(()=>parseCards('1x')).toThrow();
  const h=hand();expect(decisionReason(h)).toBe('another seat to act');
  expect(decisionReason({...h,hero_cards:null})).toBe('hero cards unknown');
  const heroTurn={...h,derived:{...h.derived,to_act:0}};
  expect(decisionReason(heroTurn)).toBeNull();
  expect(decisionReason({...heroTurn,derived:{...heroTurn.derived,legal:[{kind:'check'}]}})).toBe('fewer than two legal actions');
});

test('sparse_seats_prefill_uses_dealt_starts_and_physical_remaining',()=>{
  const h=hand({dealt:[0,2,5],stacks_start:[1000,800,1200]});
  h.derived.stacks_remaining=[975,0,750,0,0,1160];
  expect(prefill(h,{})).toEqual({0:975,2:750,5:1160});
  let w=newWizard([0,2,5],{0:975,2:750,5:1160});
  for(const key of ['1','1','Enter','Enter','Enter','Enter','Enter'])w=wizardKey(w,key);
  expect(wizardResult(w)?.stacks).toEqual([975,750,1160]);
});
