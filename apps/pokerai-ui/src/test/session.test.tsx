import { expect,test } from 'vitest';
import { render,screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ConfigScreen } from '../components/ConfigScreen';
import { readSession } from '../state/session';
import type { GameConfig } from '../ipc/types.gen';
import { config } from './fixtures';
import { FakeBackend } from './fakeBackend';
import { installMockIpc } from './mockIpc';
import { tauriBackend } from '../ipc/backend';

// Mirrors the Tauri-serialized shape of `apps/pokerai-ui/src-tauri/src/error.rs`'s `AppError`
// (`#[serde(tag = "type", content = "detail")]`) as a real `Error` subclass, so a scripted
// rejection both satisfies `@typescript-eslint/prefer-promise-reject-errors` and exercises
// `describeError`'s structural `AppErrorLike` match the same way an actual rejected `invoke(...)`
// would.
class FakeAppError extends Error {
  readonly type:string;
  readonly detail:unknown;
  constructor(type:string,detail:unknown){super(`fake AppError: ${type}`);this.name='FakeAppError';this.type=type;this.detail=detail;}
}
function formData(fields:Record<string,string|string[]>):FormData {
  const f=new FormData();
  for(const [k,v] of Object.entries(fields)) for(const value of Array.isArray(v)?v:[v]) f.append(k,value);
  return f;
}
// All six seats' stacks are explicit (R3, fix round 1): a blank stack is now a required-field
// error, not an implicit 100-BB default, so every fixture that reaches the rake/straddle/seat
// assertions below must supply a stack for each seat it selects.
const baseFields={chip_label:'$1',sb_chips:'5',bb_chips:'10',straddle:'',
  seat:['0','1','2','3','4','5'],flop_budget_s:'10',rate:'5',cap:'5',rake:'PotRake',
  'stack-0':'1000','stack-1':'1000','stack-2':'1000','stack-3':'1000','stack-4':'1000','stack-5':'1000'};

test('session_settings_validate_and_use_engine_revision',async()=>{
  const b=new FakeBackend();installMockIpc(b);const errors:unknown[]=[];
  render(<ConfigScreen config={config} stacks={{}} activeHand={true}
    save={async(c)=>{await tauriBackend.set_game_config(c);}} onError={e=>errors.push(e)}/>);
  expect(screen.getByText('Changes apply to the next hand.')).toBeVisible();
  const user=userEvent.setup();const budget=screen.getByLabelText('Flop budget (seconds)');
  expect(budget).toHaveValue(10);
  await user.clear(screen.getByLabelText('Rake cap (chips)'));
  await user.type(screen.getByLabelText('Rake cap (chips)'),'2.5');
  await user.click(screen.getByRole('button',{name:'Save session'}));
  expect(b.calls[0]).toEqual(['set_game_config',{config:{...config,
    rake:{kind:'pot_rake',rate:.05,cap_mchips:2500,no_flop_no_drop:true}}}]);
  const configForm=screen.getByTestId('config-form');
  if(!(configForm instanceof HTMLFormElement)) throw new Error('config-form is not a form element');
  const form=new FormData(configForm);
  form.set('flop_budget_s','31');expect(()=>readSession(form,config)).toThrow('1 to 30');
  expect(errors).toHaveLength(0);
});

test('session_input_edge_cases',()=>{
  const time=readSession(formData({...baseFields,rake:'TimeCharge'}),config);
  expect(time.config.rake).toEqual({kind:'time_charge'});
  const fiveSeats=formData({...baseFields,seat:['0','1','2','3','4'],straddle:'20'});
  expect(()=>readSession(fiveSeats,config)).toThrow('six seats');
  const belowTwiceBb=formData({...baseFields,straddle:'15'}); // bb=10, so 2*bb=20; six seats present
  expect(()=>readSession(belowTwiceBb,config)).toThrow('six seats');
  const capped=readSession(formData({...baseFields,cap:'2.501'}),config);
  expect(capped.config.rake).toEqual({kind:'pot_rake',rate:.05,cap_mchips:2501,no_flop_no_drop:false});
});

// R1 (fix round 1): the wire domain (`crates/proto/src/numeric.rs::domain_rake_rate`) is
// half-open `[0, 1)` -- 100% is rejected, not just values above it -- and is checked wide (f64)
// before narrowing to f32, so a rate that only overflows the domain *after* narrowing (a wide
// value the Rust reviewer reproduced as `99.999999% -> f32 1`) must also be rejected, without ever
// substituting the narrowed value into the accepted draft.
test('rake_rate_mirrors_rust_half_open_domain',()=>{
  expect(()=>readSession(formData({...baseFields,rate:'100'}),config)).toThrow();
  expect(()=>readSession(formData({...baseFields,rate:'150'}),config)).toThrow();
  expect(()=>readSession(formData({...baseFields,rate:'99.999999'}),config)).toThrow();
  const validNearBoundary=readSession(formData({...baseFields,rate:'99.99'}),config);
  if(validNearBoundary.config.rake.kind!=='pot_rake') throw new Error('expected pot_rake');
  expect(validNearBoundary.config.rake.rate).toBeCloseTo(.9999,9);
  expect(validNearBoundary.config.rake.rate).toBeLessThan(1);
  const zero=readSession(formData({...baseFields,rate:'0'}),config);
  expect(zero.config.rake).toMatchObject({kind:'pot_rake',rate:0});
});

// R3 (fix round 1, orchestrator ruling): an unspecified/blank stack is an explicit field error,
// never an implicit default -- this is a deliberate correction of the brief's original literal
// sample (see task-8-report.md's "Deviations" and the review's R3), not a fresh regression.
test('blank_stack_is_an_explicit_field_error',()=>{
  const blankSeat0=formData({...baseFields,'stack-0':''});
  expect(()=>readSession(blankSeat0,config)).toThrow(/stack for seat 1 is required/i);
});

// R2 (fix round 1): a local validation failure renders beside the field it concerns, with a real
// aria-describedby association, and never reaches `save`.
test('local_validation_error_renders_beside_its_field_and_blocks_save',async()=>{
  const errors:unknown[]=[];
  render(<ConfigScreen config={config} stacks={{}} activeHand={false}
    save={()=>Promise.reject(new Error('save must not be called for a local validation failure'))}
    onError={e=>errors.push(e)}/>);
  const user=userEvent.setup();
  const stackInput=screen.getByLabelText('Stack for seat 1');
  await user.clear(stackInput);
  await user.click(screen.getByRole('button',{name:'Save session'}));
  expect(stackInput).toHaveAttribute('aria-invalid','true');
  const describedBy=stackInput.getAttribute('aria-describedby');
  expect(describedBy).toBeTruthy();
  const message=screen.getByText(/stack for seat 1 is required/i);
  expect(message.id).toBe(describedBy);
  expect(errors).toHaveLength(1);
});

// R2 (fix round 1): a rejected `save` (the Rust command's `AppError`) is normalized into the same
// field-error representation when its message identifies a field, instead of only reaching the
// generic `onError` callback.
test('rejected_save_with_identifiable_field_renders_beside_that_field',async()=>{
  const errors:unknown[]=[];
  const rejection=new FakeAppError('Engine',{message:'UTG straddle requires six dealt seats'});
  render(<ConfigScreen config={config} stacks={{}} activeHand={false}
    save={()=>Promise.reject(rejection)} onError={e=>errors.push(e)}/>);
  await userEvent.setup().click(screen.getByRole('button',{name:'Save session'}));
  const message=await screen.findByText('UTG straddle requires six dealt seats');
  const straddleInput=screen.getByLabelText('UTG straddle (blank for none)');
  expect(straddleInput.getAttribute('aria-describedby')).toBe('straddle-error');
  expect(message.id).toBe('straddle-error');
  expect(errors).toHaveLength(1);
});

// R2 (fix round 1): a rejected `save` that cannot be associated with one field still surfaces --
// as a form-level fallback, not silently swallowed.
test('rejected_save_without_an_identifiable_field_falls_back_to_form_level',async()=>{
  const errors:unknown[]=[];
  render(<ConfigScreen config={config} stacks={{}} activeHand={false}
    save={()=>Promise.reject(new Error('boom, something unrelated failed'))} onError={e=>errors.push(e)}/>);
  await userEvent.setup().click(screen.getByRole('button',{name:'Save session'}));
  const message=await screen.findByText(/boom, something unrelated failed/i);
  expect(message.id).toBe('');
  expect(errors).toHaveLength(1);
});

// R4 (fix round 1): the rendered straddle minimum must track the big-blind draft live, so a
// same-submission "lower BB, raise straddle just above the new BB" edit is never blocked by a
// minimum computed from the configuration this screen was opened with.
test('straddle_minimum_tracks_the_current_bb_draft_live',async()=>{
  const b=new FakeBackend();installMockIpc(b);
  render(<ConfigScreen config={config} stacks={{}} activeHand={false}
    save={async(cfg)=>{await tauriBackend.set_game_config(cfg);}} onError={()=>{}}/>);
  const user=userEvent.setup();
  const bbInput=screen.getByLabelText('Big blind');
  const straddleInput=screen.getByLabelText('UTG straddle (blank for none)');
  if(!(straddleInput instanceof HTMLInputElement)) throw new Error('straddle input missing');
  expect(straddleInput.min).toBe(String(2*config.bb_chips));
  await user.clear(bbInput);
  await user.type(bbInput,'5');
  expect(straddleInput.min).toBe('10');
  await user.clear(straddleInput);
  await user.type(straddleInput,'10');
  await user.click(screen.getByRole('button',{name:'Save session'}));
  const call=b.calls[0];
  if(!call) throw new Error('expected a set_game_config call');
  expect(call[0]).toBe('set_game_config');
  const payload=call[1] as {config:GameConfig};
  expect(payload.config.bb_chips).toBe(5);
  expect(payload.config.straddle).toEqual({amount_chips:10});
});
