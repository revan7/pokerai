#![cfg(feature = "typescript")]
use proto::*;
use proto::worker::*;

/// Returns the single generated `export type NAME = ...;` line, panicking (not merely failing a
/// `contains` check) if the registry never emitted one -- a missing line is a different failure
/// mode than a wrong one, and both must be loud.
fn decl_line<'a>(text: &'a str, name: &str) -> &'a str {
    let prefix = format!("export type {name} =");
    text.lines()
        .find(|l| l.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no generated declaration line for {name}"))
}

/// R1 fix (review `task-1-review.md`): the previous version of this test only checked that a
/// declaration with the right *name* existed (`export type NAME =`), which a wrong-shaped
/// declaration (e.g. `Card` generated as `number` instead of `string`) would still satisfy. Every
/// serde witness below is now paired with an assertion against the *generated declaration text*
/// itself (via `decl_line`), so a mismatch between the Rust wire form and the TypeScript type is
/// caught here, not just a missing/renamed type.
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

    // --- Card: field-level `ts(type = "string")` override, matching Card's hand-written
    // Serialize/Deserialize (a two-character string like "As"). ---
    assert_eq!(serde_json::to_value(Card(51)).unwrap(), "As");
    assert_eq!(decl_line(&text, "Card"), "export type Card = string;",
        "Card must generate as a plain TS string; the serde witness above shows the matching wire form (\"As\")");

    // --- Seat: `#[serde(transparent)]` over `u8` must generate as a plain TS number. ---
    assert_eq!(serde_json::to_value(Seat(3)).unwrap(), serde_json::json!(3));
    assert_eq!(decl_line(&text, "Seat"), "export type Seat = number;",
        "Seat is #[serde(transparent)] over u8 and must generate as a plain TS number, not an object wrapper");

    // --- Range1326: field-level `ts(type = "Array<number>")` override. The 1326-length invariant
    // is enforced by Range1326's own Serialize/Deserialize impls (see range.rs), not something a
    // TS type alias can express, so only the element type is asserted here. ---
    assert_eq!(decl_line(&text, "Range1326"), "export type Range1326 = Array<number>;",
        "Range1326 must generate as a flat number array, matching its 1326-entry wire array");

    // --- MenuSize: hand-written `TS` impl (no `#[derive(TS)]`, since ts-rs cannot derive an
    // untagged mixed-payload enum). Both wire forms -- a pot-fraction number and the literal "a"
    // -- must appear in the one union declaration. ---
    assert_eq!(serde_json::to_string(&MenuSize::Pot(2.5)).unwrap(), "2.5");
    assert_eq!(serde_json::to_string(&MenuSize::AllIn).unwrap(), r#""a""#);
    assert_eq!(decl_line(&text, "MenuSize"), r#"export type MenuSize = number | "a";"#,
        "MenuSize's hand-written TS impl must produce both wire forms: a pot-fraction number and the literal \"a\"");

    // --- Action::AllIn vs LegalAction::AllIn: same variant name, deliberately different wire
    // tags ("allin" vs "all_in"); the generated declarations must keep them distinct. ---
    assert_eq!(serde_json::to_value(Action::AllIn { to: 10 }).unwrap(), serde_json::json!({"kind":"allin","to":10}));
    assert_eq!(serde_json::to_value(LegalAction::AllIn { to: 10 }).unwrap(), serde_json::json!({"kind":"all_in","to":10}));
    let action_decl = decl_line(&text, "Action");
    let legal_decl = decl_line(&text, "LegalAction");
    assert!(action_decl.contains(r#"{ "kind": "allin", to: number, }"#),
        "Action::AllIn must generate with the collapsed tag \"allin\": {action_decl}");
    assert!(!action_decl.contains("all_in"),
        "Action must never carry LegalAction's \"all_in\" tag spelling: {action_decl}");
    assert!(legal_decl.contains(r#"{ "kind": "all_in", to: number, }"#),
        "LegalAction::AllIn must generate with the snake_case tag \"all_in\", distinct from Action::AllIn's \"allin\": {legal_decl}");
    assert!(!legal_decl.contains(r#""allin""#),
        "LegalAction must never carry Action's \"allin\" tag spelling: {legal_decl}");

    // --- Lifecycle tag: HandPhase uses an internally-tagged `"phase"` key (not `"kind"`), lowercase
    // snake_case values. ---
    assert_eq!(serde_json::to_value(HandPhase::Betting { street: Street::Flop }).unwrap(),
        serde_json::json!({"phase":"betting","street":"flop"}));
    let hand_phase_decl = decl_line(&text, "HandPhase");
    assert!(hand_phase_decl.contains(r#"{ "phase": "betting", street: Street, }"#),
        "HandPhase::Betting must generate with the \"phase\" tag key and lowercase \"betting\" value: {hand_phase_decl}");

    // --- Event tag + flattened Final: RecommendationEvent::Final(Recommendation) is a newtype
    // variant, so on the wire it flattens to {kind:'Final', ...recommendation-fields}; in the
    // generated TypeScript this is an intersection type, `{ "kind": "Final" } & Recommendation`. ---
    let recommendation = Recommendation {
        identity: DecisionIdentity { hand_id: 1, hand_revision: 1, decision_id: 1, config_revision: 1, model_revision: 0 },
        phase: Phase::Final,
        coverage: Coverage::Exact,
        legal: vec![],
        actions: vec![],
        unresolved_mass: 0.0,
        range_mix: None,
        equity: EquitySummary::default(),
        assumptions: Assumptions {
            ranges_used: vec![], tree_signature: String::new(), template_id: String::new(),
            source: String::new(), source_accuracy: String::new(), source_granularity: String::new(),
            target_bp: 0, reached_bp: None, elapsed_ms: 0, cache: String::new(),
            translations: vec![], mappings: vec![], notes: vec![],
        },
        experimental: None,
        exploit: None,
    };
    let final_json = serde_json::to_value(RecommendationEvent::Final(recommendation)).unwrap();
    assert_eq!(final_json["kind"], serde_json::json!("Final"),
        "RecommendationEvent::Final's wire tag must be \"Final\"");
    assert_eq!(final_json["phase"], serde_json::json!("final"),
        "Final must flatten Recommendation's own fields (e.g. phase) up to the top level, not nest them under a payload key");
    let event_decl = decl_line(&text, "RecommendationEvent");
    assert!(event_decl.contains(r#"{ "kind": "Final" } & Recommendation"#),
        "RecommendationEvent::Final must generate as an intersection type flattening Recommendation's fields, matching the wire form above: {event_decl}");

    // --- Required-but-nullable vs optional-omitted worker fields: `Progress.exploitability_chips`
    // is always present on the wire (only its value may be `null`), so it must generate as
    // `T | null`, never `T?`. `Ack.reason`/`Ack.replaced`, `Result.solution`/`Result.error` and
    // `WorkerError.estimate_bytes` are `#[serde(skip_serializing_if = "Option::is_none")]` and are
    // omitted from the wire when absent, so they must generate as optional (`?:`). ---
    let progress = WorkerMessage::Progress { id: "x".into(), stage: Stage::Solving, iterations: 0, exploitability_chips: None, elapsed_ms: 0, memory_bytes: 0 };
    let progress_json = serde_json::to_value(&progress).unwrap();
    assert!(progress_json.get("exploitability_chips").is_some(),
        "exploitability_chips key must be present on the wire even when its value is None (required-but-nullable)");
    assert_eq!(progress_json["exploitability_chips"], serde_json::Value::Null);
    let ack = WorkerMessage::Ack { id: "x".into(), status: AckStatus::Accepted, reason: None, replaced: None };
    let ack_json = serde_json::to_value(&ack).unwrap();
    assert!(ack_json.get("reason").is_none(),
        "Ack.reason must be omitted from the wire when None (skip_serializing_if), matching a TS optional field");
    let worker_message_decl = decl_line(&text, "WorkerMessage");
    assert!(worker_message_decl.contains("exploitability_chips: number | null,"),
        "Progress.exploitability_chips must be required-but-nullable (T | null): {worker_message_decl}");
    assert!(!worker_message_decl.contains("exploitability_chips?:"),
        "Progress.exploitability_chips must never become optional -- the wire key is always present, only its value may be null: {worker_message_decl}");
    assert!(worker_message_decl.contains("reason?: string | null,"),
        "Ack.reason (skip_serializing_if) must generate as optional: {worker_message_decl}");
    assert!(worker_message_decl.contains("replaced?: boolean | null,"),
        "Ack.replaced (skip_serializing_if) must generate as optional: {worker_message_decl}");
    assert!(worker_message_decl.contains("solution?: StreetSolution | null,"),
        "Result.solution (skip_serializing_if) must generate as optional: {worker_message_decl}");
    assert!(worker_message_decl.contains("error?: WorkerError | null,"),
        "Result.error (skip_serializing_if) must generate as optional: {worker_message_decl}");
    let worker_error_decl = decl_line(&text, "WorkerError");
    assert!(worker_error_decl.contains("estimate_bytes?: number | null,"),
        "WorkerError.estimate_bytes (skip_serializing_if) must generate as optional: {worker_error_decl}");

    // --- Remaining tag-casing witnesses (unchanged from the original test; kept as cheap extra
    // coverage of PascalCase-vs-snake_case tag conventions across the registry). ---
    assert_eq!(serde_json::to_value(Availability::Ready).unwrap(),serde_json::json!({"kind":"Ready"}));
    assert_eq!(serde_json::to_value(Rake::TimeCharge).unwrap(),serde_json::json!({"kind":"time_charge"}));
    assert_eq!(serde_json::to_value(Action::Raise { to: 25 }).unwrap(),
        serde_json::json!({"kind":"raise","to":25}));
    assert_eq!(serde_json::to_value(Coverage::Exact).unwrap(), serde_json::json!({"kind":"Exact"}));
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
