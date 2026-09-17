//! Background pre-solver: scenario tiers and canonical-flop enumeration order (spec section
//! 10.5). The queue itself (task 14) iterates `(tier, flop_order, scenario_order)` over the
//! inventory this module defines; it is a separate task and not built here.

pub mod scenarios;
