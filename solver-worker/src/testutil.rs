//! Fixture readers shared by the in-process unit tests (§13.2's committed worker fixtures).
//!
//! A missing fixture file panics rather than skipping: a committed artifact that is absent must
//! fail the suite (plan-1 standing ruling (e)).
use proto::worker::{EngineMessage, SolveRequest};
use proto::{Action, EffectiveTree};
use serde::Deserialize;

pub fn fixture_lines(name: &str) -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/worker").join(format!("{name}.jsonl"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e} (run tools/gen_worker_fixtures.py)", path.display())).lines().filter(|l| !l.trim().is_empty()).map(String::from).collect()
}

pub fn solve_request(fixture: &str, line: usize) -> SolveRequest {
    match serde_json::from_str::<EngineMessage>(&fixture_lines(fixture)[line]).expect("parse fixture line") { EngineMessage::Solve(r) => r, other => panic!("line {line} of {fixture} is not a solve: {other:?}") }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Case { pub case: String, pub template_id: String, pub pot: u32, pub eff: u32, pub prefix: Vec<(String, Action)>, pub tree: EffectiveTree, pub history: Vec<Action>, pub decision_path: Vec<u8> }

pub fn cases() -> Vec<Case> { fixture_lines("materialization_cases").iter().map(|l| serde_json::from_str(l).expect("case")).collect() }
