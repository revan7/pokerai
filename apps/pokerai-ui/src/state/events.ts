import type { Backend } from '../ipc/backend';
import type {
  DecisionIdentity, Recommendation, RecommendationEvent, EquitySummary,
  EquityEstimate, Seat, HandState,
} from '../ipc/types.gen';

type Progress = Extract<RecommendationEvent, { kind: 'Progress' }>;

// Display state handed to the UI. `active` is the five-field decision identity the
// store currently accepts events for (spec §2, §4.4: "anything not matching the
// active identity is discarded"); `requestedAt`/`finalAt` are monotonic timestamps
// (performance.now()) used only by the e2e latency assertions, never rendered.
// `staleEventCount` (review round 1, R1) is the visible counter of every event this
// store has ever dropped for not matching the active decision -- an identity
// mismatch in `accept()`, an obsolete-generation callback delivered after
// `invalidate()`, or a buffered event abandoned because its own request went stale
// before admission landed. It is a lifetime diagnostic, not per-hand state: it
// survives `invalidate()`'s reset of the rest of `DisplayState`.
export type DisplayState = {
  active: DecisionIdentity | null;
  recommendation: Recommendation | null;
  equity: EquitySummary;
  progress: Progress | null;
  noDecision: string | null;
  requestedAt: number | null;
  finalAt: number | null;
  staleEventCount: number;
};

function emptyEquitySummary(): EquitySummary {
  return { hero_combo_vs_each: [], hero_range_vs_each: [], per_pot_shares: [] };
}

function initialState(): DisplayState {
  return {
    active: null, recommendation: null, equity: emptyEquitySummary(),
    progress: null, noDecision: null, requestedAt: null, finalAt: null,
    staleEventCount: 0,
  };
}

/** Spec §2/§4.4: every event must match all five identity fields; never infer identity. */
export function sameIdentity(a: DecisionIdentity | null, b: DecisionIdentity): boolean {
  return a !== null && a.hand_id === b.hand_id && a.hand_revision === b.hand_revision
    && a.decision_id === b.decision_id && a.config_revision === b.config_revision
    && a.model_revision === b.model_revision;
}

// MI-8 (superseded by review round 1, R2): refine-only merging. Spec §4.4's own
// text only promises a Ready estimate survives a later Pending, but review round 1
// ruled the guard must be stronger: a completed (Ready) estimate for a population
// is never erased by ANY non-Ready event for that same population -- Pending or
// Unavailable alike, including a late Unavailable arriving after Final. Only a
// newer Ready (a refined completed value) may replace a previous Ready.
function preferReady(previous: EquityEstimate | undefined, next: EquityEstimate): EquityEstimate {
  if (previous && previous.availability.kind === 'Ready' && next.availability.kind !== 'Ready') {
    return previous;
  }
  return next;
}

function mergeRows(
  previous: Array<[Seat, EquityEstimate]>,
  next: Array<[Seat, EquityEstimate]>,
): Array<[Seat, EquityEstimate]> {
  const merged = new Map(previous);
  for (const [seat, value] of next) merged.set(seat, preferReady(merged.get(seat), value));
  return [...merged];
}

// Merges by estimate population, never by array index: hero-combo, hero-range and
// each (pot_index, seat) are distinct estimates that refine independently as
// progressive `Equity` events (and the equity carried by Fast/Provisional/Final)
// arrive (spec §4.4).
export function mergeEquity(previous: EquitySummary, next: EquitySummary): EquitySummary {
  const pots = new Map(previous.per_pot_shares.map(pot => [pot.pot_index, pot] as const));
  for (const pot of next.per_pot_shares) {
    const existingShares = pots.get(pot.pot_index)?.shares ?? [];
    pots.set(pot.pot_index, { ...pot, shares: mergeRows(existingShares, pot.shares) });
  }
  return {
    hero_combo_vs_each: mergeRows(previous.hero_combo_vs_each, next.hero_combo_vs_each),
    hero_range_vs_each: mergeRows(previous.hero_range_vs_each, next.hero_range_vs_each),
    per_pot_shares: [...pots.values()],
  };
}

function eventIdentity(event: RecommendationEvent): DecisionIdentity {
  return event.identity;
}

const PHASE_RANK = { fast: 0, provisional: 1, final: 2 } as const;

// Spec §7: progress is coalesced to at most one update every 250ms.
const PROGRESS_THROTTLE_MS = 250;

export class Recommendations {
  private value = initialState();
  private generation = 0;
  private readonly listeners = new Set<() => void>();
  private progressTimer: ReturnType<typeof setTimeout> | null = null;
  private pendingProgress: Progress | null = null;

  constructor(private readonly backend: Backend, private readonly onError: (error: unknown) => void) {}

  snapshot = (): DisplayState => this.value;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };

  private publish() {
    for (const listener of this.listeners) listener();
  }

  private clearProgressTimer() {
    if (this.progressTimer !== null) clearTimeout(this.progressTimer);
    this.progressTimer = null;
    this.pendingProgress = null;
  }

  // R1: count every event this store drops for not matching the active decision --
  // an identity mismatch in `accept()`, a callback whose request generation is no
  // longer current, or a batch of events buffered for a request that went stale
  // before admission landed. `n` lets a whole abandoned buffer be counted in one
  // step instead of one call per event.
  private countStale(n = 1) {
    if (n <= 0) return;
    this.value = { ...this.value, staleEventCount: this.value.staleEventCount + n };
    this.publish();
  }

  // Spec §5 step 3 / §12: any mutation (here, a fresh request or an abandon) cancels
  // in-flight work and invalidates every descendant identity. Bumping `generation`
  // makes every callback already registered for the previous request a no-op,
  // including one delivered after this call but before its own `invoke` resolves.
  invalidate() {
    const previousActive = this.value.active;
    const staleEventCount = this.value.staleEventCount;
    this.generation += 1;
    this.clearProgressTimer();
    this.value = { ...initialState(), staleEventCount };
    this.publish();
    if (previousActive) {
      void this.backend.cancel(previousActive.decision_id).catch(this.onError);
    }
  }

  async request(hand: HandState): Promise<void> {
    this.invalidate();
    const generation = this.generation;
    const buffered: RecommendationEvent[] = [];
    let admitted = false;
    this.value = { ...this.value, requestedAt: performance.now() };
    this.publish();
    try {
      const id = await this.backend.recommend(event => {
        if (generation !== this.generation) { this.countStale(); return; }
        if (!admitted) {
          // Only the latest buffered Progress matters once admission completes.
          if (event.kind === 'Progress') {
            const existing = buffered.findIndex(candidate => candidate.kind === 'Progress');
            if (existing >= 0) buffered.splice(existing, 1);
          }
          buffered.push(event);
          return;
        }
        this.accept(event);
      });
      if (generation !== this.generation) {
        // This request was invalidated while admission was still in flight: the
        // returned identity was never adopted as `active`, so cancel it directly
        // rather than routing through `invalidate()` (which would cancel whatever
        // *is* active now instead). Every event buffered for this now-abandoned
        // request is dropped right here and must be counted (R1) -- it was
        // received, held, and never replayed into `accept()`.
        await this.backend.cancel(id.decision_id);
        this.countStale(buffered.length);
        return;
      }
      // MA-8: config_revision is deliberately excluded from this admission check.
      // Spec §4.2/§12: a session `set_game_config` bumps the session's live
      // config_revision, but `HandConfig` is frozen into the hand at `begin_hand`
      // and keeps the old value until the next hand. A mid-hand config save would
      // otherwise make `id.config_revision` (from the engine's live session
      // config) permanently mismatch `hand.config.config_revision` (frozen),
      // throwing on every subsequent request for the rest of the hand. The
      // five-field `sameIdentity()` check on individual *events* in `accept()`
      // below still guards against ever rendering the wrong decision.
      if (id.hand_id !== hand.hand_id || id.hand_revision !== hand.hand_revision) {
        await this.backend.cancel(id.decision_id);
        throw new Error('Admission identity does not match the active hand');
      }
      this.value = { ...this.value, active: id };
      admitted = true;
      for (const event of buffered) this.accept(event);
      this.publish();
    } catch (error) {
      if (generation === this.generation) {
        this.invalidate();
        this.onError(error);
      }
    }
  }

  accept(event: RecommendationEvent): void {
    if (!sameIdentity(this.value.active, eventIdentity(event))) { this.countStale(); return; }
    if (event.kind === 'NoDecision') {
      this.clearProgressTimer();
      this.value = { ...this.value, recommendation: null, noDecision: event.reason, progress: null };
    } else if (event.kind === 'Equity') {
      const equity = mergeEquity(this.value.equity, event.equity);
      this.value = {
        ...this.value, equity,
        recommendation: this.value.recommendation ? { ...this.value.recommendation, equity } : null,
      };
    } else if (event.kind === 'Progress') {
      if (this.value.recommendation?.phase === 'final' || this.value.noDecision !== null) return;
      this.pendingProgress = event;
      // MI-9: trailing-edge throttle -- the first Progress in a burst is itself
      // delayed up to 250ms, not shown immediately and then throttled. Spec §7
      // says progress is coalesced "at most every 250ms," which a trailing-edge
      // throttle satisfies; this is a deliberate simplicity choice over a
      // leading-edge throttle, which would show the very first tick sooner.
      if (this.progressTimer === null) {
        this.progressTimer = setTimeout(() => {
          this.progressTimer = null;
          const latest = this.pendingProgress;
          this.pendingProgress = null;
          if (latest && sameIdentity(this.value.active, latest.identity)) {
            this.value = { ...this.value, progress: latest };
            this.publish();
          }
        }, PROGRESS_THROTTLE_MS);
      }
      return;
    } else {
      // Fast | Provisional | Final.
      if (this.value.noDecision !== null) return;
      const recommendation: Recommendation = event;
      const current = this.value.recommendation;
      if (current && PHASE_RANK[recommendation.phase] < PHASE_RANK[current.phase]) return;
      const equity = mergeEquity(this.value.equity, recommendation.equity);
      if (recommendation.phase === 'final') this.clearProgressTimer();
      this.value = {
        ...this.value,
        recommendation: { ...recommendation, equity },
        equity,
        noDecision: null,
        progress: recommendation.phase === 'final' ? null : this.value.progress,
        finalAt: recommendation.phase === 'final' ? (this.value.finalAt ?? performance.now()) : this.value.finalAt,
      };
    }
    this.publish();
  }

  dispose(): void {
    this.invalidate();
    this.listeners.clear();
  }
}
