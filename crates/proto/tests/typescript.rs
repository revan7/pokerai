#![cfg(feature = "typescript")]
use proto::*;
#[test]
fn typescript_registry_and_wire_witnesses() {
    let text = bindings::declarations();
    for name in ["RecommendationEvent", "EffectiveTree", "MaterializedNode",
        "GameConfig", "Unavailable", "EquitySummary", "EngineMessage", "WorkerMessage"] {
        assert!(text.contains(&format!("export type {name} =")), "{name}");
    }
    for source in [include_str!("../src/cards.rs"),include_str!("../src/game.rs"),
        include_str!("../src/hand.rs"),include_str!("../src/range.rs"),
        include_str!("../src/recommendation.rs"),include_str!("../src/tree.rs"),
        include_str!("../src/worker.rs")] {
        for line in source.lines() {
            let line=line.trim();
            if let Some(rest)=line.strip_prefix("pub struct ").or_else(||line.strip_prefix("pub enum ")) {
                let name=rest.split(|c:char|!c.is_alphanumeric()&&c!='_').next().unwrap();
                assert!(text.contains(&format!("export type {name} =")),"unregistered proto type {name}");
            }
        }
    }
    assert_eq!(serde_json::to_value(LegalAction::AllIn{to:10}).unwrap(),serde_json::json!({"kind":"all_in","to":10}));
    assert_eq!(serde_json::to_value(Availability::Ready).unwrap(),serde_json::json!({"kind":"Ready"}));
    assert_eq!(serde_json::to_value(Rake::TimeCharge).unwrap(),serde_json::json!({"kind":"time_charge"}));
    assert_eq!(serde_json::to_value(Action::Raise { to: 25 }).unwrap(),
        serde_json::json!({"kind":"raise","to":25}));
    assert_eq!(serde_json::to_value(Coverage::Exact).unwrap(), serde_json::json!({"kind":"Exact"}));
    assert_eq!(serde_json::to_value(Card(51)).unwrap(), "As");
    assert_eq!(serde_json::to_value(HandPhase::Betting { street: Street::Flop }).unwrap(),
        serde_json::json!({"phase":"betting","street":"flop"}));
}

/// `declarations()` must be byte-for-byte deterministic across calls: two independent
/// registry walks (e.g. two separate `cargo run --example export_ts` invocations) must
/// never disagree, or the committed `types.gen.ts` would flap between otherwise-identical
/// builds.
#[test]
fn declarations_are_byte_for_byte_deterministic_across_calls() {
    let first = bindings::declarations();
    let second = bindings::declarations();
    assert_eq!(first, second, "bindings::declarations() must produce identical bytes on repeated calls");
}

/// The committed `apps/pokerai-ui/src/ipc/types.gen.ts` must match a fresh export exactly.
/// `export_ts.rs`'s `write_to` regenerates this file during `npm run types` (the frontend's
/// `pretest`/`dev`/`build` scripts and `src-tauri/build.rs`), so a stale committed copy would
/// only be caught here, not by the frontend build.
#[test]
fn committed_types_gen_ts_matches_a_fresh_export() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let committed_path = root.join("apps/pokerai-ui/src/ipc/types.gen.ts");
    let committed = std::fs::read_to_string(&committed_path)
        .unwrap_or_else(|e| panic!("committed types.gen.ts must exist at {committed_path:?}: {e}"));
    let fresh = bindings::declarations();
    assert_eq!(committed, fresh, "committed apps/pokerai-ui/src/ipc/types.gen.ts is stale; re-run `npm run types` (or the export_ts example) and commit the result");
}
