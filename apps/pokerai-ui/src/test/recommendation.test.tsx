import {expect,test} from 'vitest';
import {render,screen,within} from '@testing-library/react';
import {RecommendationPanel,headline} from '../components/Recommendation';
import {Equity} from '../components/Equity';
import type {DisplayState} from '../state/events';
import type {Recommendation,EquitySummary} from '../ipc/types.gen';
import {recommendation,identity,emptyEquity} from './fixtures';
import {FakeBackend} from './fakeBackend';import {installMockIpc} from './mockIpc';
const view=(r= recommendation()):DisplayState=>({active:identity,recommendation:r,equity:r.equity,
  progress:null,noDecision:null,requestedAt:10,finalAt:20,staleEventCount:0});
test('coverage_label_and_assumptions_render',()=>{
  installMockIpc(new FakeBackend());
  const r=recommendation({coverage:{kind:'Unsupported',reason:{kind:'EngineError',message:'worker crashed',retryable:true},partial:[{kind:'ChartRounded'}]}});
  render(<RecommendationPanel display={view(r)} fallbackReason={null}/>);
  expect(screen.getByTestId('coverage')).toHaveTextContent('Unsupported');
  expect(screen.getByText(/worker crashed/)).toBeVisible();expect(screen.getAllByText(/ChartRounded/).length).toBeGreaterThan(0);
  expect(screen.getByText(/source_accuracy: unverified/)).toBeVisible();
  expect(screen.getByText(/fixture-tree/)).toBeVisible();
  expect(screen.queryAllByTestId('main-ev')).toHaveLength(0);
});
test('no_decision_rendering',()=>{
  const b=new FakeBackend();installMockIpc(b);
  render(<RecommendationPanel display={{...view(),recommendation:null,noDecision:'hero all-in',equity:emptyEquity}} fallbackReason={null}/>);
  expect(screen.getByText('no decision: hero all-in')).toBeVisible();
  expect(screen.queryByRole('table',{name:'Hero actions'})).not.toBeInTheDocument();
  expect(b.calls).toHaveLength(0);
});
test('headline_rules_preserve_incomplete_and_unresolved_mass',()=>{
  const r=recommendation();expect(headline(r)).toEqual({index:1,label:'highest EV'});
  const chart={...r,actions:r.actions.map(a=>({...a,ev_bb:null,unavailable:{kind:'ChartNoEv' as const}}))};
  expect(headline(chart)?.label).toBe('highest-frequency chart action');
  expect(headline({...chart,unresolved_mass:.05})).toBeNull();
  const source:Recommendation={...chart,assumptions:{...r.assumptions,source:'PokerDataJson'},coverage:{kind:'Approximate',reasons:[{kind:'EvReferenceUnverified' as const}]}};
  expect(headline(source)?.label).toBe('highest-frequency source action, EV reference unverified');
  const incomplete:Recommendation={...r,assumptions:{...r.assumptions,source:'PokerDataJson'},actions:r.actions.map((a,i)=>
    i===0?{...a,ev_bb:null,unavailable:{kind:'BranchSupportIncomplete',covered_posterior:.2}}:a)};
  expect(headline(incomplete)?.label).toBe('highest-frequency action, EV incomplete');
  const tied={...r,actions:r.actions.map(a=>({...a,ev_bb:0,frequency:.5}))};
  expect(headline(tied)?.index).toBe(0);
});
test('unmeasured_progress_and_experimental_output_are_separate',()=>{
  installMockIpc(new FakeBackend());
  const r=recommendation();
  r.experimental={opponent:2,hero_role:'ip',pot:55,stack:975,template_id:'flop_fast_v1',
    ranges_used:[[0,'AKo',12],[2,'QQ-22',72]],actions:r.actions,reached_bp:null,elapsed_ms:100,
    note:'experimental, not solved: synthetic root, empty history, unconditioned ranges'};
  const display={...view(r),progress:{kind:'Progress' as const,identity,stage:'solving',
    iterations:0,exploitability_pct:null,elapsed_ms:50}};
  render(<RecommendationPanel display={display} fallbackReason={null}/>);
  expect(screen.getByTestId('progress')).toHaveTextContent('measuring');
  expect(screen.getByRole('complementary',{name:'experimental, not solved'})).toBeVisible();
  expect(screen.getByRole('table',{name:'Experimental actions'})).toBeVisible();
  expect(screen.getAllByTestId('main-ev')).toHaveLength(r.actions.length);
});
test('hero_out_of_support_and_deadline_best_so_far_render',()=>{
  installMockIpc(new FakeBackend());
  const r=recommendation({coverage:{kind:'Unsupported',reason:{kind:'HeroComboOutOfSupport'},partial:[]},
    actions:[], range_mix:[[{kind:'check'},.7],[{kind:'bet',to:28},.3]]});
  render(<RecommendationPanel display={view(r)} fallbackReason={null}/>);
  expect(screen.getByRole('region',{name:'Range-level mix'})).toBeVisible();
  expect(screen.queryAllByTestId('main-ev')).toHaveLength(0);
  const d=recommendation({coverage:{kind:'Approximate',reasons:[{kind:'DeadlineBestSoFar',reached_bp:190,target_bp:50}]}});
  render(<RecommendationPanel display={view(d)} fallbackReason={null}/>);
  // Scoped to the coverage-reason's own serialized fields (not a bare /190/ or /50/): the
  // fixture's default Assumptions also happen to carry target_bp:50/reached_bp:190, so an
  // unscoped text match is ambiguous between the coverage <li> and the Assumptions <p> --
  // see task-11-report.md deviation D3.
  expect(screen.getByText(/"reached_bp":190/)).toBeVisible();expect(screen.getByText(/"target_bp":50/)).toBeVisible();
});

// R1 (review round 1): the engine-computed `headline` wire flags and the UI's independently
// computed winner (via `headline()`) must agree — either the same singleton index, or both
// empty when no headline is allowed. Each fixture below sets its wire `headline` flags by hand,
// to the value an honest engine implementation would produce for that case, rather than by
// calling `headline()` to generate them; the assertion below is what would actually catch a
// real engine/UI disagreement.
test('wire_headline_flags_agree_with_the_ui_winner_or_are_both_empty',()=>{
  const wireHeadlineIndices=(r:Recommendation):number[]=>r.actions
    .map((a,i)=>a.headline?i:-1).filter(i=>i>=0);
  const uiHeadlineIndices=(r:Recommendation):number[]=>{const h=headline(r);return h?[h.index]:[];};

  // EV case: every action has a numeric EV; the higher raw ev_bb wins. Wire agrees on index 1.
  const evCase=recommendation({actions:[
    {action:{kind:'check'},frequency:.4,ev_bb:1.125,unavailable:null,headline:false},
    {action:{kind:'bet',to:28},frequency:.6,ev_bb:1.875,unavailable:null,headline:true}]});
  expect(uiHeadlineIndices(evCase)).toEqual([1]);
  expect(wireHeadlineIndices(evCase)).toEqual(uiHeadlineIndices(evCase));

  // Frequency case: no action has a numeric EV (chart source); the higher frequency wins.
  // Wire agrees on index 1.
  const freqCase=recommendation({actions:[
    {action:{kind:'check'},frequency:.3,ev_bb:null,unavailable:{kind:'ChartNoEv'},headline:false},
    {action:{kind:'bet',to:28},frequency:.7,ev_bb:null,unavailable:{kind:'ChartNoEv'},headline:true}]});
  expect(uiHeadlineIndices(freqCase)).toEqual([1]);
  expect(wireHeadlineIndices(freqCase)).toEqual(uiHeadlineIndices(freqCase));

  // Tie case: equal ev_bb and equal frequency; the earlier menu index wins. Wire agrees on index 0.
  const tieCase=recommendation({actions:[
    {action:{kind:'check'},frequency:.5,ev_bb:0,unavailable:null,headline:true},
    {action:{kind:'bet',to:28},frequency:.5,ev_bb:0,unavailable:null,headline:false}]});
  expect(uiHeadlineIndices(tieCase)).toEqual([0]);
  expect(wireHeadlineIndices(tieCase)).toEqual(uiHeadlineIndices(tieCase));

  // Unresolved case: unresolved_mass>0 forbids any headline. Wire agrees by flagging none.
  const unresolvedCase=recommendation({unresolved_mass:.05,actions:[
    {action:{kind:'check'},frequency:.4,ev_bb:1.125,unavailable:null,headline:false},
    {action:{kind:'bet',to:28},frequency:.6,ev_bb:1.875,unavailable:null,headline:false}]});
  expect(uiHeadlineIndices(unresolvedCase)).toEqual([]);
  expect(wireHeadlineIndices(unresolvedCase)).toEqual([]);

  // Unsupported case: coverage.kind==='Unsupported' forbids any headline regardless of the
  // numbers present. Wire agrees by flagging none.
  const unsupportedCase=recommendation({coverage:{kind:'Unsupported',
    reason:{kind:'EngineError',message:'worker crashed',retryable:true},partial:[]},actions:[
    {action:{kind:'check'},frequency:.5,ev_bb:1,unavailable:null,headline:false},
    {action:{kind:'bet',to:28},frequency:.5,ev_bb:2,unavailable:null,headline:false}]});
  expect(uiHeadlineIndices(unsupportedCase)).toEqual([]);
  expect(wireHeadlineIndices(unsupportedCase)).toEqual([]);

  // Negative witness: same tie-break math as tieCase, but the wire deliberately flags the wrong
  // index. This proves the agreement assertion above would actually reject a real disagreement,
  // rather than vacuously passing regardless of the wire's flags.
  const mismatched=recommendation({actions:[
    {action:{kind:'check'},frequency:.5,ev_bb:0,unavailable:null,headline:false},
    {action:{kind:'bet',to:28},frequency:.5,ev_bb:0,unavailable:null,headline:true}]});
  expect(uiHeadlineIndices(mismatched)).toEqual([0]);
  expect(wireHeadlineIndices(mismatched)).toEqual([1]);
  expect(wireHeadlineIndices(mismatched)).not.toEqual(uiHeadlineIndices(mismatched));
});

// R2 (review round 1): every Equity availability branch must render distinctly (Pending and
// Unavailable are never confused with a numeric 0%), Monte Carlo standard error must convert to
// percentage points, and the three populations (hero-combo, hero-range, per-pot) must each show
// their own text. Separately, a main action with exactly-zero frequency and exactly-zero EV must
// still render as an explicit zero, not be suppressed as unavailable.
test('equity_availability_branches_render_distinctly_including_exact_zero',()=>{
  const value:EquitySummary={
    hero_combo_vs_each:[[1,{value:0,availability:{kind:'Ready'},method:{kind:'Exact'}}]],
    hero_range_vs_each:[[2,{value:.5,availability:{kind:'Ready'},
      method:{kind:'MonteCarlo',samples:20000,std_err:.01}}]],
    per_pot_shares:[
      {pot_index:0,population:'range vs range, main pot',
        shares:[[3,{value:null,availability:{kind:'Pending'},method:null}]]},
      {pot_index:1,population:'range vs range, side pot 1',
        shares:[[4,{value:null,availability:{kind:'Unavailable',reason:'multiway not solved'},method:null}]]}]};
  render(<Equity value={value}/>);
  // Ready/Exact zero renders as an explicit numeric zero, not blank or "unavailable".
  expect(screen.getByText('0.00% · Exact')).toBeVisible();
  // Ready/MonteCarlo shows samples and converts std_err to percentage points (100 * std_err).
  expect(screen.getByText('50.00% · MonteCarlo; samples 20000; std error 1.000 pp')).toBeVisible();
  // Pending is distinct from a numeric zero -- never rendered as "0.00%".
  expect(screen.getByText('pending')).toBeVisible();
  expect(screen.queryByText(/0\.00%.*pending/)).not.toBeInTheDocument();
  // Unavailable carries its own reason and is distinct from both pending and zero.
  expect(screen.getByText('unavailable: multiway not solved')).toBeVisible();
  // The three populations are independently labelled and visible.
  expect(screen.getByText('Hero combo versus each opponent')).toBeVisible();
  expect(screen.getByText('Hero public range versus each public range')).toBeVisible();
  expect(within(screen.getByRole('region',{name:'Pot 1 equity'})).getByText('range vs range, main pot')).toBeVisible();
  expect(within(screen.getByRole('region',{name:'Pot 2 equity'})).getByText('range vs range, side pot 1')).toBeVisible();

  installMockIpc(new FakeBackend());
  const r=recommendation({actions:[
    {action:{kind:'check'},frequency:0,ev_bb:0,unavailable:null,headline:false},
    {action:{kind:'bet',to:28},frequency:1,ev_bb:1.5,unavailable:null,headline:true}]});
  render(<RecommendationPanel display={view(r)} fallbackReason={null}/>);
  const zeroRow=screen.getByText('check').closest('tr') as HTMLElement;
  expect(within(zeroRow).getByText('0.0%')).toBeVisible();
  const zeroEv=within(zeroRow).getByTestId('main-ev');
  expect(zeroEv).toHaveTextContent('0.00 bb');
  expect(zeroEv).toHaveAttribute('data-ev-bb','0');
});
