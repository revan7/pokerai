import { expect, test, vi } from 'vitest';
import type { DecisionIdentity, EquityEstimate, EquitySummary, RecommendationEvent } from '../ipc/types.gen';
import { mergeEquity, Recommendations } from '../state/events';
import { FakeBackend } from './fakeBackend';
import { hand, identity, recommendation, emptyEquity } from './fixtures';
import { installMockIpc } from './mockIpc';
import { tauriBackend } from '../ipc/backend';

test('stale_identity_discarded', async () => {
  const b = new FakeBackend(); installMockIpc(b); const store = new Recommendations(tauriBackend, () => {});
  b.beforeIdentity = [{kind:'Final',...recommendation()}];
  await store.request(hand());
  expect(store.snapshot().recommendation?.phase).toBe('final');
  expect(store.snapshot().staleEventCount).toBe(0);
  let expectedStale = 0;
  for (const field of ['hand_id','hand_revision','decision_id','config_revision','model_revision'] as const) {
    const stale = {...identity,[field]:identity[field]+1};
    b.emit({kind:'Final',...recommendation({identity:stale,coverage:{kind:'Exact'}})});
    b.emit({kind:'NoDecision',identity:stale,reason:'stale'});
    b.emit({kind:'Equity',identity:stale,equity:emptyEquity});
    b.emit({kind:'Progress',identity:stale,stage:'stale',iterations:1,exploitability_pct:1,elapsed_ms:1});
    expectedStale += 4;
    expect(store.snapshot().recommendation?.coverage.kind).not.toBe('Exact');
    expect(store.snapshot().noDecision).toBeNull();
    // R1: every one of the four events above was dropped by accept()'s identity
    // check (one mutated field each) and must be counted, one per rejected event.
    expect(store.snapshot().staleEventCount).toBe(expectedStale);
  }
  store.invalidate(); b.emit({kind:'Final',...recommendation()});
  expect(store.snapshot().recommendation).toBeNull();
  // R1: this Final targets the now-invalidated request's own obsolete-generation
  // callback -- a different discard path than the identity mismatches above -- and
  // must also be counted.
  expect(store.snapshot().staleEventCount).toBe(expectedStale + 1);
  b.beforeIdentity = []; b.nextIdentity = {...identity,hand_id:11,decision_id:91};
  await store.request(hand({hand_id:11}));
  b.emit({kind:'Final',...recommendation()},0);
  expect(store.snapshot().recommendation).toBeNull();
  expect(store.snapshot().staleEventCount).toBe(expectedStale + 2);
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

test('obsolete_admission_is_cancelled_and_never_adopted', async () => {
  const fake = new FakeBackend(); installMockIpc(fake);
  let sink!: (e: RecommendationEvent) => void;
  let resolveAdmission!: (id: DecisionIdentity) => void;
  const held = new Promise<DecisionIdentity>(done => { resolveAdmission = done; });
  // R3: capture the sink the store actually registered instead of discarding it --
  // the previous stub replaced `recommend` with a function that ignored its sink
  // argument, so no Final could ever be delivered or buffered through it.
  vi.spyOn(fake, 'recommend').mockImplementationOnce(s => { sink = s; return held; });
  const store = new Recommendations(tauriBackend, () => {});

  const request = store.request(hand());
  // An event delivered on the just-issued sink before admission resolves must be
  // buffered, never adopted as the active identity.
  sink({kind:'Final',...recommendation()});
  expect(store.snapshot().active).toBeNull();
  expect(store.snapshot().recommendation).toBeNull();

  store.invalidate();
  resolveAdmission(identity);
  await request;
  // The buffered Final belonged to a request abandoned before its own admission
  // landed: it must stay hidden, and the returned decision id must be cancelled.
  expect(store.snapshot().active).toBeNull();
  expect(store.snapshot().recommendation).toBeNull();
  expect(fake.calls).toContainEqual(['cancel', {decision_id: identity.decision_id}]);
  // R1: the abandoned buffered Final is itself a discarded/stale event and counted.
  expect(store.snapshot().staleEventCount).toBe(1);

  // A newer active request must be unaffected by the obsolete resolution: delivering
  // on the old sink now must not clear or cancel the new request's active identity.
  fake.nextIdentity = {...identity, hand_id:11, decision_id:91};
  await store.request(hand({hand_id:11}));
  sink({kind:'Final',...recommendation()});
  expect(store.snapshot().active).toEqual(fake.nextIdentity);
  expect(store.snapshot().recommendation).toBeNull();
  expect(store.snapshot().staleEventCount).toBe(2);
  fake.emit({kind:'Final',...recommendation({identity: fake.nextIdentity})});
  expect(store.snapshot().recommendation?.phase).toBe('final');
  expect(store.snapshot().active).toEqual(fake.nextIdentity);
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

test('ready_survives_unavailable_after_final_per_population', async () => {
  const fake = new FakeBackend(); installMockIpc(fake); const store = new Recommendations(tauriBackend, () => {});
  await store.request(hand());
  const ready: EquityEstimate = {value:.6, availability:{kind:'Ready'}, method:{kind:'Exact'}};
  const pending: EquityEstimate = {value:null, availability:{kind:'Pending'}, method:null};
  const unavailable: EquityEstimate = {value:null, availability:{kind:'Unavailable', reason:'worker_error'}, method:null};
  fake.emit({kind:'Equity', identity, equity:{
    ...emptyEquity,
    hero_combo_vs_each:[[2,ready]],
    hero_range_vs_each:[[2,ready]],
    per_pot_shares:[{pot_index:0, population:'hero combo fixed', shares:[[0,ready],[2,ready]]}],
  }});
  fake.emit({kind:'Final', ...recommendation({equity:{
    ...emptyEquity,
    hero_combo_vs_each:[[2,pending]],
    hero_range_vs_each:[[2,pending]],
    per_pot_shares:[{pot_index:0, population:'hero combo fixed', shares:[[2,pending]]}],
  }})});
  expect(store.snapshot().recommendation?.equity.hero_combo_vs_each[0]?.[1]).toEqual(ready);
  expect(store.snapshot().recommendation?.equity.hero_range_vs_each[0]?.[1]).toEqual(ready);
  expect(store.snapshot().equity.per_pot_shares[0]?.shares).toEqual([[0,ready],[2,ready]]);
  // R2: a later Unavailable, delivered as an independent Equity event AFTER the
  // Final above, must also not erase the completed estimates -- review round 1's
  // stronger rule. Spec §4.4's own text only names Pending; this project's ruling
  // extends the non-regression guarantee to every non-Ready kind.
  fake.emit({kind:'Equity', identity, equity:{
    ...emptyEquity,
    hero_combo_vs_each:[[2,unavailable]],
    hero_range_vs_each:[[2,unavailable]],
    per_pot_shares:[{pot_index:0, population:'hero combo fixed', shares:[[2,unavailable]]}],
  }});
  expect(store.snapshot().recommendation?.equity.hero_combo_vs_each[0]?.[1]).toEqual(ready);
  expect(store.snapshot().recommendation?.equity.hero_range_vs_each[0]?.[1]).toEqual(ready);
  expect(store.snapshot().equity.per_pot_shares[0]?.shares).toEqual([[0,ready],[2,ready]]);
  store.dispose();
});

test('mergeEquity_prefers_ready_over_every_non_ready_transition', () => {
  const estimate = (kind: 'Ready' | 'Pending' | 'Unavailable'): EquityEstimate =>
    kind === 'Ready' ? {value:.5, availability:{kind:'Ready'}, method:{kind:'Exact'}}
    : kind === 'Pending' ? {value:null, availability:{kind:'Pending'}, method:null}
    : {value:null, availability:{kind:'Unavailable', reason:'worker_error'}, method:null};
  const summary = (e: EquityEstimate): EquitySummary => ({...emptyEquity, hero_combo_vs_each:[[2,e]]});
  const kinds = ['Ready', 'Pending', 'Unavailable'] as const;
  for (const previous of kinds) {
    for (const next of kinds) {
      const merged = mergeEquity(summary(estimate(previous)), summary(estimate(next)));
      // Only a previous Ready followed by a non-Ready next is guarded; every other
      // transition (including Ready refining a previous Ready) takes the next value.
      const expected = previous === 'Ready' && next !== 'Ready' ? estimate(previous) : estimate(next);
      expect(merged.hero_combo_vs_each[0]?.[1]).toEqual(expected);
    }
  }
  for (const next of kinds) {
    const merged = mergeEquity(emptyEquity, summary(estimate(next)));
    expect(merged.hero_combo_vs_each[0]?.[1]).toEqual(estimate(next));
  }
});
