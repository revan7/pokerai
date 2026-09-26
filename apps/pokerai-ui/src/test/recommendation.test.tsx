import {expect,test} from 'vitest';
import {render,screen} from '@testing-library/react';
import {RecommendationPanel,headline} from '../components/Recommendation';
import type {DisplayState} from '../state/events';
import type {Recommendation} from '../ipc/types.gen';
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
