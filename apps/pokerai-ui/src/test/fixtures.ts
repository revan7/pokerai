import type { GameConfig, HandState, DecisionIdentity, Recommendation, EquitySummary } from '../ipc/types.gen';

export const config: GameConfig = { config_revision: 1, chip_label: '$1', sb_chips: 5,
  bb_chips: 10, straddle: null,
  rake: {kind:'pot_rake', rate: .05, cap_mchips: 5000, no_flop_no_drop: true },
  seats: [0,1,2,3,4,5].map(seat => ({ seat, tag: null, facts: [] })),
  solver: { threads: 16, target_bp: 50, flop_budget_s: 10 } };

export const identity: DecisionIdentity = { hand_id: 10, hand_revision: 7,
  decision_id: 90, config_revision: 1, model_revision: 0 };

export const emptyEquity: EquitySummary = { hero_combo_vs_each: [], hero_range_vs_each: [], per_pot_shares: [] };

export function hand(patch: Partial<HandState> = {}): HandState {
  return { hand_id: 10, hand_revision: 7, config: {
    config_revision: 1, chip_label: '$1', sb_chips: 5, bb_chips: 10,
    straddle: null, rake: config.rake }, phase: {phase:'betting', street: 'preflop' },
    button: 0, hero: 0, hero_cards: ['As','Kd'], dealt: [0,1,2,3,4,5],
    stacks_start: [1000,1000,1000,1000,1000,1000], board: [], actions: [],
    derived: { street: 'preflop', to_act: 3, pot: 15,
      committed_this_street: [0,5,10,0,0,0], stacks_remaining: [1000,995,990,1000,1000,1000],
      folded: [false,false,false,false,false,false], all_in: [false,false,false,false,false,false],
      facing: 10, last_full_raise: 10, pots: [],
      legal: [{kind:'fold'}, {kind:'call',cost:10}, {kind:'raise',min_to:20,max_to:1000}, {kind:'all_in',to:1000}] },
    ...patch };
}

export function recommendation(patch: Partial<Recommendation> = {}): Recommendation {
  return { identity, phase: 'final', coverage: {kind:'Approximate', reasons: [{kind:'ChartRounded'}] },
    legal: [{kind:'check'}, {kind:'bet',min_to:10,max_to:975}, {kind:'all_in',to:975}],
    actions: [{ action: {kind:'check'}, frequency:.4, ev_bb:1.125, unavailable:null, headline:false },
      { action:{kind:'bet',to:28}, frequency:.6, ev_bb:1.875, unavailable:null, headline:true }],
    unresolved_mass:0, range_mix:null, equity:emptyEquity,
    assumptions: { ranges_used:[[0,'AKo:1',12],[2,'JJ-22,AQs-A2s',120]],
      tree_signature:'fixture-tree', template_id:'flop_fast_v1', source:'ChartTranscription',
      source_accuracy:'unverified', source_granularity:'169 classes; rounded, no EV',
      target_bp:50, reached_bp:190, elapsed_ms:9850, cache:'miss',
      translations:[], mappings:[], notes:['mode=i16; realized bets 28 chips'] },
    experimental:null, exploit:null, ...patch };
}
