//! The façade `bench` uses instead of depending on `core-*` directly (spec §3.2 dependency direction).
//! Plan 4 extends this module; it never grows a second entry point.
use proto::{Card, Range1326};

/// Parses a Pio-style range string and applies board blocking, exactly as the engine does at a street root.
pub fn prepared_range(text: &str, board: &[Card]) -> Result<Range1326, String> {
    let mut r = core_ranges::parse_range(text).map_err(|e| format!("range {text:?}: {e}"))?;
    core_ranges::block_public(&mut r, board);
    Ok(r)
}
/// Total weight of a range, for report and sanity checks.
pub fn range_mass(r: &Range1326) -> f32 { core_ranges::mass(r) }

/// Spec 4.5 `SolveRequest.spot`: the lowercase sha256 hex of the structural identity of the game `req` solves, the
/// string a staged lock is matched against. Two requests that solve the same game get the same identity, and every input
/// that makes two games differ enters it:
/// - the §4.6 `tree_signature` of `req.tree` at `req.pot` (rules version, template, root street, nominal menus,
///   thresholds, wager cap, inserted sizes as reduced rationals of the root pot);
/// - the chip scale the signature deliberately leaves out, which fixes the realized tree: `pot`, `stack_oop` and
///   `stack_ip` (two spots that differ only in their stacks, such as a 100bb and a 200bb spot on one board, are
///   different games with different all-in nodes), and the rake (`rake_rate` bits, `-0.0` read as `0.0`, and
///   `rake_cap_mchips`);
/// - the board's card ids in order, and the §2 scaled hashes of `oop_range` then `ip_range`.
///
/// Solve parameters are not the game and never enter it: `id`, `spot` itself, `history` (the requested node),
/// `target_bp`, `deadline_ms`, `extraction_margin_ms`, `memory_limit_bytes`, `background`. The preimage is
/// domain-tagged and every part is fixed-width except the board, whose length is written first.
///
/// Plan 2 Task 22's listing sketches `engine::solve::spot_hash(tree, pot, board, ranges)` without the stacks and the
/// rake, which would give the two spots above one identity; `bench` reaches this helper through the façade so it never
/// depends on `core-*` (§3.2).
///
/// Panics as `tree_signature` does (a zero pot, an inserted path that does not resolve): both are engine bugs.
pub fn spot_identity(req: &proto::worker::SolveRequest) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"pokerai/solve-spot/v1\0");
    h.update(crate::tree::tree_signature(&req.tree, req.pot).as_bytes());
    for chips in [req.pot, req.stack_oop, req.stack_ip] {
        h.update(chips.to_le_bytes());
    }
    let rake_rate = if req.rake_rate == 0.0 { 0.0f32 } else { req.rake_rate };
    h.update(rake_rate.to_bits().to_le_bytes());
    h.update(req.rake_cap_mchips.to_le_bytes());
    let board_len = u8::try_from(req.board.len()).expect("a board has at most five cards");
    h.update([board_len]);
    for card in &req.board {
        h.update([card.0]);
    }
    h.update(core_ranges::hash_scaled(&req.oop_range));
    h.update(core_ranges::hash_scaled(&req.ip_range));
    hex::encode(h.finalize())
}

// --- Task 17: the frozen chart provenance/source lock (`bench/spots/sources.json`) ---
//
// `tools/chart_sources.py::freeze_sources` writes this shape from the committed chart bundles.
// `SourceBundle`/`UnavailableBundle` only declare the fields this crate actually reads (`name`,
// `sha256`, `bytes`, `nodes`, `reason`) -- the JSON also carries `source`/`manifest` provenance
// metadata for human/Python consumers, which serde silently ignores here (no
// `deny_unknown_fields`: adding a field on the Python side must never break the Rust reader).

/// One available chart bundle's frozen identity: its content hash, byte length, and the
/// canonical node-history keys (`tools/chart_sources.py::history_key`) it covers.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SourceBundle {
    pub name: String,
    pub sha256: String,
    pub bytes: u64,
    pub nodes: Vec<String>,
}

/// A depth Plan 3's acquisition record (`fixtures/charts/sources.manifest.json`) marked anything
/// other than `"available"` -- never substituted with another publisher, depth or screenshot.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct UnavailableBundle {
    pub name: String,
    pub reason: String,
}

/// The whole frozen lock: every available bundle's identity, every unavailable depth's reason,
/// and the declared synthetic-missing node list (spots no chart source covers at all).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SourceLock {
    pub version: u16,
    pub snapshot_date: String,
    pub missing: Vec<String>,
    pub bundles: Vec<SourceBundle>,
    pub unavailable: Vec<UnavailableBundle>,
}

/// Reads and structurally validates `sources.json`: well-formed JSON matching `SourceLock`'s
/// shape (a missing required field, or a wrongly-typed one, is a decode error, never silently
/// defaulted), and `version == 1` -- the only version this crate understands. A corrupt or
/// future-versioned lock is rejected here, before any bundle hash is ever trusted.
pub fn load_source_lock(path: &std::path::Path) -> Result<SourceLock, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let lock: SourceLock =
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    if lock.version != 1 {
        return Err(format!(
            "{}: sources.json version must be 1, found {}",
            path.display(),
            lock.version
        ));
    }
    Ok(lock)
}

// --- Task 17 fix round (I1): the acquisition record `verify_source_lock` reconciles the lock
// against -- only the fields this crate reads (`bundle_id`, `status`, `note`); no
// `deny_unknown_fields`, same rationale as `SourceBundle`/`UnavailableBundle` above.

#[derive(Debug, serde::Deserialize)]
struct AcquisitionDepth {
    bundle_id: String,
    status: String,
    #[serde(default)]
    #[allow(dead_code)] // read for completeness/documentation; not compared against yet
    note: String,
}

#[derive(Debug, serde::Deserialize)]
struct AcquisitionRecord {
    depths: Vec<AcquisitionDepth>,
}

/// Letter-per-step history key, mirroring `tools/chart_sources.py::history_key` exactly (fold=F,
/// check=X, call=C, raise=R, allin=A; position and amount dropped -- see that function's
/// docstring for why the letter sequence alone is already a canonical key). Used only to
/// recompute a chart envelope's own node inventory from its raw JSON so `verify_source_lock` can
/// compare it against the frozen lock's `nodes` list without trusting that list on its own.
fn history_key_from_json(history: &serde_json::Value) -> Result<String, String> {
    let steps = history
        .as_array()
        .ok_or_else(|| "a chart node's history must be a JSON array".to_string())?;
    let mut key = String::with_capacity(steps.len());
    for step in steps {
        let letter = match step.get(1).and_then(|v| v.as_str()) {
            Some("fold") => 'F',
            Some("check") => 'X',
            Some("call") => 'C',
            Some("raise") => 'R',
            Some("allin") => 'A',
            Some(other) => return Err(format!("unrecognized history step {other:?}")),
            None => return Err("a history entry is missing its step field".to_string()),
        };
        key.push(letter);
    }
    Ok(key)
}

/// Parses `raw` (an already-read `<name>.json` chart envelope) and returns the canonical node key
/// for every node it declares -- erroring, never skipping, on a malformed node so drift is never
/// silently under-counted.
fn chart_node_keys(raw: &[u8], name: &str) -> Result<Vec<String>, String> {
    let envelope: serde_json::Value =
        serde_json::from_slice(raw).map_err(|e| format!("{name}: {e}"))?;
    let nodes = envelope
        .get("nodes")
        .and_then(|n| n.as_array())
        .ok_or_else(|| format!("{name}: chart envelope has no 'nodes' array"))?;
    nodes
        .iter()
        .map(|node| {
            let history = node
                .get("history")
                .ok_or_else(|| format!("{name}: a chart node is missing 'history'"))?;
            history_key_from_json(history).map_err(|e| format!("{name}: {e}"))
        })
        .collect()
}

/// Recomputes each available bundle's sha256 (and byte length) from the chart file the store
/// actually loaded (`charts_dir/<name>.json`), so a chart that was re-transcribed, hand-edited or
/// replaced after the freeze aborts generation instead of silently benchmarking stale data. A
/// bundle file that cannot even be read (missing, permission-denied) is reported by name rather
/// than panicking.
///
/// Also reconciles the lock's `bundles`/`unavailable` identities against Plan 3 Task 4's
/// acquisition record (`charts_dir/sources.manifest.json`) -- every acquired depth must be
/// present in exactly one of the two lists, with a supported status and (for `unavailable`) a
/// non-empty reason, and every name in either list must be backed by an acquisition row -- and,
/// for every available bundle, compares its declared `nodes` inventory against the chart
/// envelope's actual node histories in both directions. A lock that silently dropped, added or
/// duplicated a bundle or a node is rejected here, before Task 20 (or anything else) trusts it.
/// Before this fix (task-17 fix round I1), the structural-only checks here accepted a corrupted
/// inventory -- including an empty `bundles: []` lock -- as long as its declared bundles' hashes
/// happened to match; `bundle.nodes` was never read at all, and an acquired-but-omitted bundle
/// went undetected.
pub fn verify_source_lock(lock: &SourceLock, charts_dir: &std::path::Path) -> Result<(), String> {
    use sha2::Digest;
    use std::collections::HashSet;

    // --- Reconcile available/unavailable bundle identities against the acquisition record ---
    let acquisition_path = charts_dir.join("sources.manifest.json");
    let acquisition_bytes = std::fs::read(&acquisition_path)
        .map_err(|e| format!("{}: {e}", acquisition_path.display()))?;
    let acquisition: AcquisitionRecord = serde_json::from_slice(&acquisition_bytes)
        .map_err(|e| format!("{}: {e}", acquisition_path.display()))?;

    let mut lock_available: HashSet<&str> = HashSet::new();
    for bundle in &lock.bundles {
        if !lock_available.insert(bundle.name.as_str()) {
            return Err(format!(
                "{}: duplicate bundle name in sources.json's bundles",
                bundle.name
            ));
        }
    }
    let mut lock_unavailable: HashSet<&str> = HashSet::new();
    for entry in &lock.unavailable {
        if !lock_unavailable.insert(entry.name.as_str()) {
            return Err(format!(
                "{}: duplicate bundle name in sources.json's unavailable list",
                entry.name
            ));
        }
        if entry.reason.trim().is_empty() {
            return Err(format!(
                "{}: an unavailable entry in sources.json has an empty reason",
                entry.name
            ));
        }
        if lock_available.contains(entry.name.as_str()) {
            return Err(format!(
                "{}: listed as both an available bundle and unavailable in sources.json",
                entry.name
            ));
        }
    }

    let mut acquisition_names: HashSet<&str> = HashSet::new();
    for row in &acquisition.depths {
        if !acquisition_names.insert(row.bundle_id.as_str()) {
            return Err(format!(
                "{}: duplicate bundle_id in sources.manifest.json",
                row.bundle_id
            ));
        }
        match row.status.as_str() {
            "available" => {
                if !lock_available.contains(row.bundle_id.as_str()) {
                    return Err(format!(
                        "{}: sources.manifest.json marks this bundle available but sources.json's bundles list is missing it",
                        row.bundle_id
                    ));
                }
            }
            "unsupported" => {
                if !lock_unavailable.contains(row.bundle_id.as_str()) {
                    return Err(format!(
                        "{}: sources.manifest.json marks this bundle unsupported but sources.json's unavailable list is missing it",
                        row.bundle_id
                    ));
                }
            }
            other => {
                return Err(format!(
                    "{}: sources.manifest.json has an unsupported status {other:?}",
                    row.bundle_id
                ));
            }
        }
    }
    for name in lock_available.iter().chain(lock_unavailable.iter()) {
        if !acquisition_names.contains(name) {
            return Err(format!(
                "{name}: present in sources.json but absent from sources.manifest.json"
            ));
        }
    }

    // --- Per-bundle content verification: bytes, and the node inventory in both directions ---
    for bundle in &lock.bundles {
        let path = charts_dir.join(format!("{}.json", bundle.name));
        let raw = std::fs::read(&path).map_err(|e| format!("{}: {e}", bundle.name))?;
        let actual_hash = hex::encode(sha2::Sha256::digest(&raw));
        if actual_hash != bundle.sha256 || raw.len() as u64 != bundle.bytes {
            return Err(format!("{} drifted from the frozen source lock", bundle.name));
        }

        let actual_keys = chart_node_keys(&raw, &bundle.name)?;
        let mut actual_set: HashSet<&str> = HashSet::new();
        for key in &actual_keys {
            if !actual_set.insert(key.as_str()) {
                return Err(format!(
                    "{}: the chart envelope itself has a duplicate node history {key:?}",
                    bundle.name
                ));
            }
        }
        let mut lock_set: HashSet<&str> = HashSet::new();
        for key in &bundle.nodes {
            if !lock_set.insert(key.as_str()) {
                return Err(format!(
                    "{}: duplicate node {key:?} in sources.json's node inventory",
                    bundle.name
                ));
            }
        }
        let mut missing: Vec<&str> = actual_set
            .iter()
            .filter(|k| !lock_set.contains(**k))
            .copied()
            .collect();
        missing.sort_unstable();
        if !missing.is_empty() {
            return Err(format!(
                "{}: the chart envelope has node(s) {missing:?} missing from sources.json's node inventory",
                bundle.name
            ));
        }
        let mut extra: Vec<&str> = lock_set
            .iter()
            .filter(|k| !actual_set.contains(**k))
            .copied()
            .collect();
        extra.sort_unstable();
        if !extra.is_empty() {
            return Err(format!(
                "{}: sources.json's node inventory has node(s) {extra:?} not present in the chart envelope",
                bundle.name
            ));
        }
    }
    Ok(())
}

// --- Task 19: the fifty recorded e2e hands (`fixtures/hands/e2e/001.json`..`050.json` +
// `manifest.json`, generated and frozen by `tools/gen_fixtures.py e2e` from `tools/e2e_hands.py`)
// ---
//
// This is the Rust-side benchmark input format (Task 21 translates it into proto's actual wire
// commands); it intentionally does not depend on `proto`'s JSON field layout. Hero's cards are
// carried on `hero_cards` for hero-combo equity and terminal calculations only -- `load_records`
// never derives a public range or cache key from a record, and this module has no way to.

/// One event of a recorded hand's history: either an action (`seat`/`street`/`action` present,
/// `cards` absent) or a board deal (`cards` present, the rest absent). `action` is kept as raw
/// JSON (`{"kind": ..., "to": ...}`) -- Task 21 maps it into proto's action wire type; this crate
/// does not need to interpret it to load and hash-verify the record.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct RecordedEvent {
    #[serde(rename = "type")]
    pub kind: String,
    pub seat: Option<u8>,
    pub street: Option<String>,
    pub action: Option<serde_json::Value>,
    pub cards: Option<String>,
}

/// One recorded e2e hand, exactly `tools/e2e_hands.py::records()`'s per-record v1 schema. `config`
/// and `expected` are kept as raw JSON (nested shapes Task 21/26 read field-by-field); everything
/// else is typed because `load_records`'s callers need it directly (seat count, stacks, hero seat
/// and cards, the event list to replay).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct RecordedHand {
    pub id: String,
    pub version: u16,
    pub config: serde_json::Value,
    pub button: u8,
    pub dealt: Vec<u8>,
    pub stacks: Vec<u32>,
    pub hero: u8,
    pub hero_cards: String,
    pub events: Vec<RecordedEvent>,
    pub fault: Option<String>,
    pub class: String,
    pub expected: serde_json::Value,
}

/// Loads `fixtures/hands/e2e/001.json`..`050.json`, verifying every file's sha256 against
/// `manifest.json` before returning any of them -- a byte drift (a hand hand-edited outside the
/// generator, or a stale copy left over from a partial regeneration) is an error, never a silent
/// load of mismatched data. The manifest itself is not a hand: this loads exactly 50 records, in
/// `001`..`050` order, regardless of the manifest's own key order.
pub fn load_records(dir: &std::path::Path) -> Result<Vec<RecordedHand>, String> {
    use sha2::Digest;
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dir.join("manifest.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let hashes = manifest["sha256"]
        .as_object()
        .ok_or_else(|| "manifest.json has no 'sha256' object".to_string())?;
    if hashes.len() != 50 {
        return Err(format!("expected 50 records, found {}", hashes.len()));
    }
    let mut out = Vec::new();
    for n in 1..=50 {
        let name = format!("{n:03}.json");
        let raw = std::fs::read(dir.join(&name)).map_err(|e| format!("{name}: {e}"))?;
        let expected = hashes
            .get(&name)
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("{name}: no hash in manifest.json"))?;
        let actual = hex::encode(sha2::Sha256::digest(&raw));
        if actual != expected {
            return Err(format!("{name} does not match the frozen hash in manifest.json"));
        }
        out.push(
            serde_json::from_slice::<RecordedHand>(&raw).map_err(|e| format!("{name}: {e}"))?,
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::Card;
    #[test]
    fn prepared_range_parses_and_blocks_the_board() {
        let board: Vec<Card> = ["Kh", "7d", "2c"].iter().map(|s| Card::parse(s).unwrap()).collect();
        let r = prepared_range("AA,KK", &board).unwrap();
        assert_eq!((range_mass(&r) * 1000.0).round() as u32, 9000);   // 6 aces + 3 kings (Kh is on the board)
        assert!(prepared_range("not a range", &board).is_err());
    }
}

#[cfg(test)]
mod recorded_hand_tests {
    use super::*;
    use std::path::Path;

    fn repo_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// A fresh, uniquely-tagged directory, mirroring `source_lock_tests::temp_dir` above.
    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pokerai_recorded_hand_tests_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A minimal, structurally valid `RecordedHand` JSON document for record `id` -- every field
    /// `RecordedHand` requires, values that do not need to mean anything beyond deserializing.
    fn minimal_record_json(id: &str) -> Vec<u8> {
        format!(
            r#"{{"id":"{id}","version":1,"config":{{}},"button":0,"dealt":[0,1,2,3,4,5],
                "stacks":[10000,10000,10000,10000,10000,10000],"hero":2,"hero_cards":"6s6h",
                "events":[{{"type":"board","cards":"Kh7d2c"}}],"fault":null,"class":"hu_flop_srp",
                "expected":{{"numeric_ev":true}}}}"#
        )
        .into_bytes()
    }

    /// Writes a manifest whose `sha256` map has exactly 50 entries (`001.json`..`050.json`), all
    /// but any names in `overrides` mapped to a filler hash string that this function never
    /// verifies against real file bytes -- callers that need loading to succeed past a given
    /// record must instead provide that record's real hash via `overrides`.
    fn write_manifest_50(dir: &Path, overrides: &std::collections::HashMap<String, String>) {
        let mut hashes = serde_json::Map::new();
        for n in 1..=50u32 {
            let name = format!("{n:03}.json");
            let hash = overrides.get(&name).cloned().unwrap_or_else(|| "0".repeat(64));
            hashes.insert(name, serde_json::Value::String(hash));
        }
        let manifest = serde_json::json!({"version": 1, "synthetic": true, "sha256": hashes});
        std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    }

    #[test]
    fn load_records_reads_the_committed_fifty() {
        let records = load_records(&repo_root().join("fixtures/hands/e2e")).unwrap();
        assert_eq!(records.len(), 50);
        assert_eq!(records[0].id, "001");
        assert_eq!(records[49].id, "050");
        assert!(records.iter().all(|r| r.version == 1));
        assert!(records.iter().all(|r| r.dealt.len() == 6 && r.stacks.len() == 6));
        let numeric = records
            .iter()
            .filter(|r| r.expected.get("numeric_ev") == Some(&serde_json::Value::Bool(true)))
            .count();
        assert_eq!(numeric, 22);
        // hero's cards are carried on the record only -- confirms the field exists and is never
        // itself a range/key type that could be mistaken for one.
        assert!(records.iter().all(|r| r.hero_cards.len() == 4));
    }

    #[test]
    fn load_records_rejects_a_manifest_with_the_wrong_record_count() {
        let dir = temp_dir("wrong_count");
        let mut hashes = serde_json::Map::new();
        hashes.insert("001.json".into(), serde_json::Value::String("0".repeat(64)));
        let manifest = serde_json::json!({"version": 1, "sha256": hashes});
        std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
        let err = load_records(&dir).unwrap_err();
        assert!(err.contains("expected 50 records, found 1"), "{err}");
    }

    #[test]
    fn load_records_rejects_a_file_that_does_not_match_its_frozen_hash() {
        let dir = temp_dir("bad_hash");
        write_manifest_50(&dir, &std::collections::HashMap::new());
        std::fs::write(dir.join("001.json"), minimal_record_json("001")).unwrap();
        let err = load_records(&dir).unwrap_err();
        assert!(err.contains("001.json does not match the frozen hash"), "{err}");
    }

    #[test]
    fn load_records_rejects_a_missing_file() {
        use sha2::Digest;
        let dir = temp_dir("missing_file");
        let raw = minimal_record_json("001");
        let hash = hex::encode(sha2::Sha256::digest(&raw));
        let mut overrides = std::collections::HashMap::new();
        overrides.insert("001.json".to_string(), hash);
        write_manifest_50(&dir, &overrides);
        std::fs::write(dir.join("001.json"), &raw).unwrap();
        // 002.json is declared in the manifest (with a filler hash) but never written.
        let err = load_records(&dir).unwrap_err();
        assert!(err.contains("002.json"), "{err}");
    }

    #[test]
    fn load_records_rejects_a_manifest_missing_the_sha256_object() {
        let dir = temp_dir("no_sha256");
        std::fs::write(dir.join("manifest.json"), r#"{"version":1}"#).unwrap();
        let err = load_records(&dir).unwrap_err();
        assert!(err.contains("sha256"), "{err}");
    }
}

#[cfg(test)]
mod source_lock_tests {
    use super::*;
    use std::path::Path;

    fn repo_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn temp_file(name: &str, contents: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pokerai_source_lock_tests_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    /// A fresh, uniquely-tagged directory for a single test (task-17 fix round I1's new tests
    /// each write their own small `sources.manifest.json` and/or synthetic chart file, so each
    /// needs isolation from the others -- unlike `temp_file` above, whose callers already use
    /// distinct filenames within one shared directory).
    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("pokerai_source_lock_tests_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Writes a minimal synthetic `<name>.json` chart envelope containing one node per entry of
    /// `node_histories` (each a `history_key`-style letter string, e.g. `"FF"`); the seat name is
    /// irrelevant to `chart_node_keys` (only the step letter is read), so every step uses a
    /// placeholder seat. Returns the written bytes' (sha256, length) for building a matching
    /// `SourceBundle`.
    fn write_synthetic_bundle(dir: &Path, name: &str, node_histories: &[&str]) -> (String, u64) {
        use sha2::Digest;
        let steps = |letters: &str| -> String {
            letters
                .chars()
                .map(|c| {
                    let step = match c {
                        'F' => "fold",
                        'X' => "check",
                        'C' => "call",
                        'R' => "raise",
                        'A' => "allin",
                        other => panic!("unsupported test letter {other:?}"),
                    };
                    format!(r#"["X","{step}",0]"#)
                })
                .collect::<Vec<_>>()
                .join(",")
        };
        let nodes: Vec<String> = node_histories
            .iter()
            .map(|h| format!(r#"{{"history":[{}]}}"#, steps(h)))
            .collect();
        let raw = format!(r#"{{"nodes":[{}]}}"#, nodes.join(",")).into_bytes();
        std::fs::write(dir.join(format!("{name}.json")), &raw).unwrap();
        let hash = hex::encode(sha2::Sha256::digest(&raw));
        (hash, raw.len() as u64)
    }

    /// Writes a minimal `sources.manifest.json` acquisition record with one row per
    /// `(bundle_id, status)` pair -- only the fields `AcquisitionDepth` reads.
    fn write_acquisition_record(dir: &Path, rows: &[(&str, &str)]) {
        let depths: Vec<String> = rows
            .iter()
            .map(|(id, status)| format!(r#"{{"bundle_id":"{id}","status":"{status}","note":""}}"#))
            .collect();
        std::fs::write(
            dir.join("sources.manifest.json"),
            format!(r#"{{"depths":[{}]}}"#, depths.join(",")),
        )
        .unwrap();
    }

    #[test]
    fn load_source_lock_reads_the_committed_lock() {
        let lock = load_source_lock(&repo_root().join("bench/spots/sources.json")).unwrap();
        assert_eq!(lock.version, 1);
        assert_eq!(lock.snapshot_date, "2026-09-10");
        assert_eq!(lock.missing, vec!["UTG-limp", "CO-limp", "BB-cold-call-vs-3bet"]);
        let names: Vec<&str> = lock.bundles.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, vec!["pokercoaching_100", "rangeconverter_200"]);
        assert!(lock.bundles.iter().all(|b| b.sha256.len() == 64));
        assert!(lock.bundles.iter().any(|b| b.nodes.iter().any(|n| n == "FFFF")));
    }

    #[test]
    fn verify_source_lock_passes_against_the_committed_charts() {
        let lock = load_source_lock(&repo_root().join("bench/spots/sources.json")).unwrap();
        verify_source_lock(&lock, &repo_root().join("fixtures/charts")).unwrap();
    }

    #[test]
    fn verify_source_lock_rejects_a_drifted_hash() {
        let mut lock = load_source_lock(&repo_root().join("bench/spots/sources.json")).unwrap();
        lock.bundles[0].sha256 = "0".repeat(64);
        let err = verify_source_lock(&lock, &repo_root().join("fixtures/charts")).unwrap_err();
        assert!(err.contains("drifted"), "{err}");
    }

    #[test]
    fn verify_source_lock_rejects_a_byte_length_mismatch() {
        let mut lock = load_source_lock(&repo_root().join("bench/spots/sources.json")).unwrap();
        lock.bundles[0].bytes += 1;
        let err = verify_source_lock(&lock, &repo_root().join("fixtures/charts")).unwrap_err();
        assert!(err.contains("drifted"), "{err}");
    }

    #[test]
    fn verify_source_lock_rejects_a_missing_chart_file() {
        // A directory that has the acquisition record and every OTHER required chart file, but
        // is specifically missing `pokercoaching_100.json` -- isolates "this one bundle's file
        // vanished" from "the whole charts directory is gone" (the next test).
        let dir = temp_dir("missing_one_file");
        std::fs::copy(
            repo_root().join("fixtures/charts/sources.manifest.json"),
            dir.join("sources.manifest.json"),
        )
        .unwrap();
        std::fs::copy(
            repo_root().join("fixtures/charts/rangeconverter_200.json"),
            dir.join("rangeconverter_200.json"),
        )
        .unwrap();
        let lock = load_source_lock(&repo_root().join("bench/spots/sources.json")).unwrap();
        let err = verify_source_lock(&lock, &dir).unwrap_err();
        assert!(err.contains("pokercoaching_100"), "{err}");
    }

    #[test]
    fn verify_source_lock_rejects_a_nonexistent_charts_directory() {
        let lock = load_source_lock(&repo_root().join("bench/spots/sources.json")).unwrap();
        let err = verify_source_lock(&lock, Path::new("no/such/charts/dir")).unwrap_err();
        assert!(err.contains("sources.manifest.json"), "{err}");
    }

    // --- Task 17 fix round I1: full bidirectional inventory reconciliation ---

    #[test]
    fn verify_source_lock_accepts_an_available_and_an_unsupported_bundle() {
        // The exact scenario the brief names: a legitimately unsupported depth (200bb here,
        // standing in for RangeConverter's conditional-availability rule) must still pass when
        // it is backed by its own acquisition row and correctly recorded as `unavailable`.
        let dir = temp_dir("accepts_unsupported");
        let (hash, bytes) = write_synthetic_bundle(&dir, "pokercoaching_100", &["", "F"]);
        write_acquisition_record(
            &dir,
            &[("pokercoaching_100", "available"), ("rangeconverter_200", "unsupported")],
        );
        let lock = SourceLock {
            version: 1,
            snapshot_date: "x".into(),
            missing: vec![],
            bundles: vec![SourceBundle {
                name: "pokercoaching_100".into(),
                sha256: hash,
                bytes,
                nodes: vec!["".into(), "F".into()],
            }],
            unavailable: vec![UnavailableBundle {
                name: "rangeconverter_200".into(),
                reason: "depth 200 unsupported by the acquired sources".into(),
            }],
        };
        verify_source_lock(&lock, &dir).unwrap();
    }

    #[test]
    fn verify_source_lock_rejects_an_empty_lock_against_the_real_acquisition_record() {
        // The exact bypass the review demonstrated: a structurally valid but empty lock used to
        // pass verification outright. It must now be rejected because the real acquisition
        // record marks both depths available.
        let lock = SourceLock {
            version: 1,
            snapshot_date: "x".into(),
            missing: vec![],
            bundles: vec![],
            unavailable: vec![],
        };
        let err = verify_source_lock(&lock, &repo_root().join("fixtures/charts")).unwrap_err();
        assert!(err.contains("missing it"), "{err}");
    }

    #[test]
    fn verify_source_lock_rejects_an_available_bundle_missing_from_sources_json() {
        let dir = temp_dir("missing_available");
        let (hash, bytes) = write_synthetic_bundle(&dir, "pokercoaching_100", &[""]);
        write_acquisition_record(
            &dir,
            &[("pokercoaching_100", "available"), ("rangeconverter_200", "available")],
        );
        let lock = SourceLock {
            version: 1,
            snapshot_date: "x".into(),
            missing: vec![],
            bundles: vec![SourceBundle {
                name: "pokercoaching_100".into(),
                sha256: hash,
                bytes,
                nodes: vec!["".into()],
            }],
            unavailable: vec![],
        };
        let err = verify_source_lock(&lock, &dir).unwrap_err();
        assert!(err.contains("rangeconverter_200"), "{err}");
        assert!(err.contains("missing it"), "{err}");
    }

    #[test]
    fn verify_source_lock_rejects_a_bundle_marked_both_available_and_unavailable() {
        let dir = temp_dir("both_lists");
        let (hash, bytes) = write_synthetic_bundle(&dir, "pokercoaching_100", &[""]);
        write_acquisition_record(&dir, &[("pokercoaching_100", "available")]);
        let lock = SourceLock {
            version: 1,
            snapshot_date: "x".into(),
            missing: vec![],
            bundles: vec![SourceBundle {
                name: "pokercoaching_100".into(),
                sha256: hash,
                bytes,
                nodes: vec!["".into()],
            }],
            unavailable: vec![UnavailableBundle {
                name: "pokercoaching_100".into(),
                reason: "dup".into(),
            }],
        };
        let err = verify_source_lock(&lock, &dir).unwrap_err();
        assert!(err.contains("both an available bundle and unavailable"), "{err}");
    }

    #[test]
    fn verify_source_lock_rejects_duplicate_bundle_names() {
        let dir = temp_dir("dup_bundle");
        let (hash, bytes) = write_synthetic_bundle(&dir, "pokercoaching_100", &[""]);
        write_acquisition_record(&dir, &[("pokercoaching_100", "available")]);
        let bundle = SourceBundle {
            name: "pokercoaching_100".into(),
            sha256: hash,
            bytes,
            nodes: vec!["".into()],
        };
        let lock = SourceLock {
            version: 1,
            snapshot_date: "x".into(),
            missing: vec![],
            bundles: vec![bundle.clone(), bundle],
            unavailable: vec![],
        };
        let err = verify_source_lock(&lock, &dir).unwrap_err();
        assert!(err.contains("duplicate bundle name"), "{err}");
    }

    #[test]
    fn verify_source_lock_rejects_a_bundle_absent_from_the_acquisition_record() {
        let dir = temp_dir("ghost_bundle");
        let (hash, bytes) = write_synthetic_bundle(&dir, "ghost_bundle", &[""]);
        write_acquisition_record(&dir, &[]);
        let lock = SourceLock {
            version: 1,
            snapshot_date: "x".into(),
            missing: vec![],
            bundles: vec![SourceBundle {
                name: "ghost_bundle".into(),
                sha256: hash,
                bytes,
                nodes: vec!["".into()],
            }],
            unavailable: vec![],
        };
        let err = verify_source_lock(&lock, &dir).unwrap_err();
        assert!(err.contains("ghost_bundle"), "{err}");
        assert!(err.contains("absent from sources.manifest.json"), "{err}");
    }

    #[test]
    fn verify_source_lock_rejects_a_missing_node_in_the_lock() {
        let dir = temp_dir("missing_node");
        let (hash, bytes) = write_synthetic_bundle(&dir, "pokercoaching_100", &["", "F"]);
        write_acquisition_record(&dir, &[("pokercoaching_100", "available")]);
        let lock = SourceLock {
            version: 1,
            snapshot_date: "x".into(),
            missing: vec![],
            bundles: vec![SourceBundle {
                name: "pokercoaching_100".into(),
                sha256: hash,
                bytes,
                nodes: vec!["".into()], // "F" is a real node the lock never declares
            }],
            unavailable: vec![],
        };
        let err = verify_source_lock(&lock, &dir).unwrap_err();
        assert!(err.contains("missing from sources.json's node inventory"), "{err}");
        assert!(err.contains("\"F\""), "{err}");
    }

    #[test]
    fn verify_source_lock_rejects_an_extra_node_in_the_lock() {
        let dir = temp_dir("extra_node");
        let (hash, bytes) = write_synthetic_bundle(&dir, "pokercoaching_100", &[""]);
        write_acquisition_record(&dir, &[("pokercoaching_100", "available")]);
        let lock = SourceLock {
            version: 1,
            snapshot_date: "x".into(),
            missing: vec![],
            bundles: vec![SourceBundle {
                name: "pokercoaching_100".into(),
                sha256: hash,
                bytes,
                nodes: vec!["".into(), "F".into()], // "F" does not exist in the chart envelope
            }],
            unavailable: vec![],
        };
        let err = verify_source_lock(&lock, &dir).unwrap_err();
        assert!(err.contains("not present in the chart envelope"), "{err}");
    }

    #[test]
    fn verify_source_lock_rejects_a_duplicate_node_in_the_lock() {
        let dir = temp_dir("dup_node");
        let (hash, bytes) = write_synthetic_bundle(&dir, "pokercoaching_100", &[""]);
        write_acquisition_record(&dir, &[("pokercoaching_100", "available")]);
        let lock = SourceLock {
            version: 1,
            snapshot_date: "x".into(),
            missing: vec![],
            bundles: vec![SourceBundle {
                name: "pokercoaching_100".into(),
                sha256: hash,
                bytes,
                nodes: vec!["".into(), "".into()],
            }],
            unavailable: vec![],
        };
        let err = verify_source_lock(&lock, &dir).unwrap_err();
        assert!(err.contains("duplicate node"), "{err}");
    }

    #[test]
    fn load_source_lock_rejects_wrong_version() {
        let path = temp_file(
            "bad_version.json",
            r#"{"version":2,"snapshot_date":"x","missing":[],"bundles":[],"unavailable":[]}"#,
        );
        let err = load_source_lock(&path).unwrap_err();
        assert!(err.contains("version"), "{err}");
    }

    #[test]
    fn load_source_lock_rejects_malformed_json() {
        let path = temp_file("corrupt.json", "{ not json");
        assert!(load_source_lock(&path).is_err());
    }

    #[test]
    fn load_source_lock_rejects_a_lock_missing_a_required_field() {
        // no `bundles` key at all -- must be a decode error, never a silently-defaulted empty vec.
        let path = temp_file(
            "missing_field.json",
            r#"{"version":1,"snapshot_date":"x","missing":[],"unavailable":[]}"#,
        );
        assert!(load_source_lock(&path).is_err());
    }

    #[test]
    fn load_source_lock_rejects_a_missing_file() {
        assert!(load_source_lock(Path::new("no/such/file.json")).is_err());
    }
}
