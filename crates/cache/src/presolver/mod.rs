//! Background pre-solver (spec section 10.5): the scenario tiers and canonical-flop enumeration
//! order (`scenarios`, task 13) and the durable, resumable job queue persisted as `queue.json`,
//! iterated in `(tier, flop_order, scenario_order)`, with progress keyed by each slot's normalized
//! game identity, retry deadlines persisted as UTC, and completion verified by reading the cache
//! entry back (`queue`, task 14). The scheduler thread that drives the queue is task 15.

pub mod queue;
pub mod scenarios;
