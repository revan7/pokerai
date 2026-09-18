import { expect, test, vi } from 'vitest';
import type { DecisionIdentity, EquityEstimate } from '../ipc/types.gen';
import { Recommendations } from '../state/events';
import { FakeBackend } from './fakeBackend';
import { hand, identity, recommendation, emptyEquity } from './fixtures';
import { installMockIpc } from './mockIpc';
import { tauriBackend } from '../ipc/backend';

test('stale_identity_discarded', async () => {
  const b = new FakeBackend(); installMockIpc(b); const store = new Recommendations(tauriBackend, () => {});
  b.beforeIdentity = [{kind:'Final',...recommendation()}];
  await store.request(hand());
  expect(store.snapshot().recommendation?.phase).toBe('final');
  for (const field of ['hand_id','hand_revision','decision_id','config_revision','model_revision'] as const) {
    const stale = {...identity,[field]:identity[field]+1};
    b.emit({kind:'Final',...recommendation({identity:stale,coverage:{kind:'Exact'}})});
    b.emit({kind:'NoDecision',identity:stale,reason:'stale'});
    b.emit({kind:'Equity',identity:stale,equity:emptyEquity});
    b.emit({kind:'Progress',identity:stale,stage:'stale',iterations:1,exploitability_pct:1,elapsed_ms:1});
    expect(store.snapshot().recommendation?.coverage.kind).not.toBe('Exact');
    expect(store.snapshot().noDecision).toBeNull();
  }
  store.invalidate(); b.emit({kind:'Final',...recommendation()});
  expect(store.snapshot().recommendation).toBeNull();
  b.beforeIdentity = []; b.nextIdentity = {...identity,hand_id:11,decision_id:91};
  await store.request(hand({hand_id:11}));
  b.emit({kind:'Final',...recommendation()},0);
  expect(store.snapshot().recommendation).toBeNull();
  store.dispose();
});

test('progressive_fast_equity_provisional_final', async () => {
  vi.useFakeTimers();
  try {
    const b=new FakeBackend();installMockIpc(b);const store=new Recommendations(tauriBackend,()=>{});
    await store.request(hand());
    b.emit({kind:'Fast',...recommendation({phase:'fast'})});
    b.emit({kind:'Progress',identity,stage:'solving',iterations:0,exploitability_pct:null,elapsed_ms:10});
    vi.advanceTimersByTime(250);
    expect(store.snapshot().progress?.exploitability_pct).toBeNull();
    b.emit({kind:'Provisional',...recommendation({phase:'provisional'})});
    b.emit({kind:'Final',...recommendation()});
    b.emit({kind:'Equity',identity,equity:{...emptyEquity,hero_combo_vs_each:[[2,
      {value:.62,availability:{kind:'Ready'},method:{kind:'MonteCarlo',samples:10000,std_err:.0048}}]]}});
    expect(store.snapshot().recommendation?.phase).toBe('final');
    expect(store.snapshot().recommendation?.equity.hero_combo_vs_each[0]?.[1].value).toBe(.62);
    b.emit({kind:'Equity',identity,equity:{...emptyEquity,hero_combo_vs_each:[[2,
      {value:null,availability:{kind:'Pending'},method:null}]]}});
    b.emit({kind:'Fast',...recommendation({phase:'fast'})});
    expect(store.snapshot().recommendation?.phase).toBe('final');
    expect(store.snapshot().equity.hero_combo_vs_each[0]?.[1].availability.kind).toBe('Ready');
    store.dispose();
  } finally {vi.useRealTimers();}
});

test('obsolete_admission_is_cancelled_and_never_adopted',async()=>{
  const fake=new FakeBackend();installMockIpc(fake);
  let resolve!:(id:DecisionIdentity)=>void;
  const held=new Promise<DecisionIdentity>(done=>{resolve=done;});
  fake.recommend=vi.fn(()=>held);
  const store=new Recommendations(tauriBackend,()=>{});
  const request=store.request(hand());store.invalidate();
  resolve(identity);await request;
  expect(store.snapshot().active).toBeNull();
  expect(fake.calls).toContainEqual(['cancel',{decision_id:identity.decision_id}]);
  store.dispose();
});

test('ready_before_final_survives_pending_per_population',async()=>{
  const fake=new FakeBackend();installMockIpc(fake);const store=new Recommendations(tauriBackend,()=>{});
  await store.request(hand());
  const ready:EquityEstimate={value:.6,availability:{kind:'Ready'},method:{kind:'Exact'}};
  const pending:EquityEstimate={value:null,availability:{kind:'Pending'},method:null};
  fake.emit({kind:'Equity',identity,equity:{...emptyEquity,hero_range_vs_each:[[2,ready]],
    per_pot_shares:[{pot_index:0,population:'hero combo fixed',shares:[[0,ready],[2,ready]]}]}});
  fake.emit({kind:'Final',...recommendation({equity:{...emptyEquity,hero_range_vs_each:[[2,pending]],
    per_pot_shares:[{pot_index:0,population:'hero combo fixed',shares:[[2,pending]]}]}})});
  expect(store.snapshot().recommendation?.equity.hero_range_vs_each[0]?.[1]).toEqual(ready);
  expect(store.snapshot().equity.per_pot_shares[0]?.shares).toEqual([[0,ready],[2,ready]]);
  store.dispose();
});
