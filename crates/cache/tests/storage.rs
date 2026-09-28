//! Bounded binary storage for the on-disk street-solution cache (spec section 10.4; plan 4 task
//! 5, fix round 1 per `task-5-review.md` R1-R6): `Cell`, `encode`/`decode`, `entry_path`,
//! `read_cell`. A cell holds one or two `CacheEntry` values sharing the same structural key; on
//! disk it is a fixed 56-byte header (magic, schema/proto version, compressed/decoded length,
//! decoded-payload sha256) followed by a zstd-compressed bincode payload -- the whole entry,
//! metadata and nodes together, in one lossless bincode encoding (review R1; see
//! `crates/cache/src/storage.rs`'s module doc for why the brief's original JSON-metadata split
//! is not lossless and was replaced). Every bound (cell entry count, key agreement, header size
//! fields, decoded/compressed byte ceilings) is checked before the corresponding allocation or
//! decode call, and any read/decode failure is a cache miss with best-effort, identity-checked
//! deletion of the offending file (review R3; CLAUDE.md section 6 "Read/decode error: miss,
//! best-effort delete").

mod support;
// The per-invocation temp directory (review R5; one shared file since task 6 review R7):
// exclusively created, unique per process and call, removed on `Drop` -- including while a
// failing assertion unwinds -- and clearing a dead process's stale leftover before it is created.
#[path = "support/temp_dir.rs"]
mod temp_dir;

use bincode::Options;
use cache::entry::{validate_entry, CacheEntry};
use cache::storage::{self, Cell, COMPRESSED_MAX, DECODED_MAX};
use sha2::{Digest, Sha256};
use std::io::Read;
use temp_dir::TempDir;

fn assert_miss_and_deleted(bytes: &[u8], dir: &std::path::Path, name: &str) {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    assert!(storage::read_cell(&path).is_none(), "{name} must be a cache miss");
    assert!(!path.exists(), "{name} must be deleted after a failed read");
}

// --- shared byte-level framing helpers (mirrors `cache::storage::encode`'s framing; review R2/R4) --

fn test_options() -> impl Options {
    bincode::DefaultOptions::new().with_fixint_encoding().with_little_endian().with_limit(DECODED_MAX).reject_trailing_bytes()
}

/// Recompresses `payload` and rebuilds a full 56-byte header + compressed body around it,
/// exactly matching `encode`'s own framing -- used to rebuild a well-formed, correctly
/// checksummed file around a payload that was tampered with directly, after a real `encode()`
/// run, rather than through any checked struct serializer (review R4: "Patch native f32 bytes in
/// a decompressed valid payload... then recompute compression, lengths, and sha256").
fn reframe(payload: &[u8]) -> Vec<u8> {
    let compressed = zstd::stream::encode_all(payload, 3).unwrap();
    let mut out = Vec::with_capacity(56 + compressed.len());
    out.extend_from_slice(b"PAI3");
    out.extend_from_slice(&3_u16.to_le_bytes());
    out.extend_from_slice(&3_u16.to_le_bytes());
    out.extend_from_slice(&(compressed.len() as u64).to_le_bytes());
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(&Sha256::digest(payload));
    out.extend_from_slice(&compressed);
    out
}

fn decompressed_payload_of(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    zstd::stream::read::Decoder::new(&bytes[56..]).unwrap().read_to_end(&mut out).unwrap();
    out
}

/// Mirrors `cache::storage::encode`'s on-disk framing exactly, except it never calls
/// `cache::entry::validate_entry` -- used only to get a structurally well-formed, correctly
/// checksummed file onto disk whose *content* is invalid, so a subsequent `read_cell` miss can
/// be attributed to content validation, not just the checksum (review R4). `CacheEntry`
/// (including its nested `CachedNode` matrices) still goes through its own checked `Serialize`,
/// so this cannot carry a bare out-of-domain or nonfinite scalar -- those are exercised through
/// byte-level payload patching instead (`reframe`/`decompressed_payload_of` above). Not exported
/// from `cache` itself (brief step 5: "Do not export an unchecked writer from cache").
fn write_unchecked(entries: &[CacheEntry]) -> Vec<u8> {
    let borrowed: Vec<&CacheEntry> = entries.iter().collect();
    let payload = test_options().serialize(&borrowed).expect("mutated-but-in-domain entries must still bincode-serialize");
    reframe(&payload)
}

fn capped_decompress(compressed: &[u8]) -> Vec<u8> {
    let mut reader = zstd::stream::read::Decoder::new(compressed).unwrap();
    reader.window_log_max(28).unwrap();
    let mut payload = Vec::new();
    reader.take(DECODED_MAX + 1).read_to_end(&mut payload).unwrap();
    payload
}

// --- brief step 1, verbatim ---------------------------------------------------------------------

#[test]
fn cache_corrupt_entry_deleted() {
    let dir = TempDir::new("corrupt");
    let path = dir.path().join("bad.bin");
    std::fs::write(&path, b"not a cache header").unwrap();
    assert!(cache::storage::read_cell(&path).is_none());
    assert!(!path.exists());
    // A directory cannot be removed as a file: deletion failure is still a miss.
    assert!(cache::storage::read_cell(dir.path()).is_none());
}

// --- task 6: atomic publication (`write_atomic`) -------------------------------------------------

/// Brief step 1's replacement test and step 5's "after rename: the new cell is complete", entirely
/// inside one per-invocation `TempDir` (review R7: no global, PID-only path, and cleanup on
/// unwind). Step 5's "inject failure before rename" runs the *production* `write_atomic` with a
/// failure injected between its temp write and its rename, which only a crate-internal
/// `#[cfg(test)]` seam can do -- see `crates/cache/src/storage.rs`'s unit tests (review R6); the
/// crash leftovers a killed writer would leave are `sweep_temporaries`'s test in
/// `crates/cache/tests/quota.rs`.
#[test]
fn cache_atomic_write_and_quota() {
    let dir = TempDir::new("atomic");
    let path = dir.path().join("pokerai-atomic.bin");
    cache::storage::write_atomic(&path, b"old").unwrap();
    cache::storage::write_atomic(&path, b"new").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"new");

    // Step 5, "after rename": a real cell replacing a real cell lands whole, and leaves no temp
    // file of its own behind.
    let e = support::entry();
    let published = storage::entry_path(dir.path(), e.key.digest());
    let old = storage::encode(&Cell { entries: vec![e.clone()] }).unwrap();
    storage::write_atomic(&published, &old).unwrap();
    let mut replacement_entry = support::entry();
    replacement_entry.iterations = 4_321;
    let new = storage::encode(&Cell { entries: vec![replacement_entry] }).unwrap();
    assert_ne!(new, old, "the replacement must be distinguishable from the old cell");
    storage::write_atomic(&published, &new).unwrap();
    assert_eq!(std::fs::read(&published).unwrap(), new, "the replacement lands whole");
    assert_eq!(storage::read_cell(&published).unwrap().entries[0].iterations, 4_321);
    assert_eq!(std::fs::read_dir(published.parent().unwrap()).unwrap().count(), 1, "write_atomic leaves no temp file of its own behind");
}

/// Review R7: the shared helper never inherits a previous run's files -- a stale leftover at a
/// fresh name (a dead process that had this process id) is cleared before the exclusive create --
/// and removes its directory on `Drop`.
#[test]
fn temp_dir_clears_a_stale_leftover_and_removes_itself_on_drop() {
    let parent = TempDir::new("temp-dir-parent");
    let stale = parent.path().join("stale");
    std::fs::create_dir(&stale).unwrap();
    std::fs::write(stale.join("leftover.bin"), b"a dead run's file").unwrap();

    let fresh = TempDir::fresh(stale.clone());
    assert_eq!(fresh.path(), stale);
    assert_eq!(std::fs::read_dir(fresh.path()).unwrap().count(), 0, "a fresh TempDir starts empty, never adopting a leftover");
    drop(fresh);
    assert!(!stale.exists(), "a TempDir removes its directory on drop");
}

/// A publication that cannot complete reports the failure and cleans up after itself: the
/// caller's recommendation path only ever sees an error, never a stray temp file and never a
/// partially replaced cell.
#[test]
fn write_atomic_reports_a_failed_publication_and_removes_its_temp_file() {
    let dir = TempDir::new("atomic-fail");
    let blocked = dir.path().join("cell.bin");
    std::fs::create_dir(&blocked).unwrap(); // rename cannot replace a directory
    assert!(storage::write_atomic(&blocked, b"payload").is_err(), "publishing over a directory must fail");
    assert!(blocked.is_dir(), "the failed publication must not have disturbed what was there");
    let leftovers = std::fs::read_dir(dir.path()).unwrap().flatten().filter(|f| f.path().extension().and_then(|e| e.to_str()) == Some("tmp")).count();
    assert_eq!(leftovers, 0, "a failed publication removes its own temp file");

    assert!(storage::write_atomic(std::path::Path::new(""), b"payload").is_err(), "a path with no parent directory is rejected");
}

// --- encode: cell-level invariants ---------------------------------------------------------------

/// review m4: entries that disagree on their own structural key have no business sharing a cell
/// -- `encode` must reject this before ever validating or writing either entry. Fix round 1
/// (review R2): the agreement check is now a direct `KeyFields` equality, not a
/// `KeyFields::digest()` call, so this also exercises that path.
#[test]
fn cell_key_mismatch_rejected_by_encode() {
    let a = support::entry();
    let mut b = support::entry();
    b.key.spr_bucket += 1;
    assert_ne!(a.key, b.key);
    let cell = Cell { entries: vec![a, b] };
    assert!(storage::encode(&cell).is_err());
}

#[test]
fn cell_entry_count_out_of_range_rejected_by_encode() {
    assert!(storage::encode(&Cell { entries: vec![] }).is_err(), "an empty cell");
    let e = support::entry();
    assert!(storage::encode(&Cell { entries: vec![e.clone(), e.clone(), e] }).is_err(), "three entries");
}

/// Brief step 5's literal parameter cases: an in-memory `CacheEntry` whose probability row for a
/// populated combo is out of domain, or sums to something other than 1, or whose `covered_paths`
/// are reordered against `nodes`, must fail `validate_entry` directly -- `-0.1`/`1.1` cannot even
/// reach a byte stream through any *checked* serializer (`CachedNode`'s own `Serialize` enforces
/// the `[0, 1]` domain on every format, including bincode; see `crates/cache/src/entry.rs`'s
/// `native_checked`). Review R4: they *can* still reach a byte stream through direct byte-level
/// patching of an already-encoded payload, which is why
/// `cache_storage_rejects_and_deletes_byte_level_corrupt_payloads` below exercises the same two
/// values again, through `read_cell` this time -- this test's job is only the in-memory check.
///
/// Plan 4 Task 8: every mutation runs on both fixtures -- Task 2's hand-built `support::entry()` and the real
/// materialized fixture (`real_entry()`, the production `flop_fast_v1` tree Task 8 froze) -- and each fixture is
/// itself valid first, so every rejection is the mutation's.
#[test]
fn cache_payload_validated() {
    for (fixture, entry) in fixtures() {
        assert!(validate_entry(&entry).is_ok(), "{fixture}: the unmutated fixture is valid");
        for row in [vec![0.2, 0.2], vec![-0.1, 1.1], vec![1.1, -0.1]] {
            let mut e = entry.clone();
            let i = e.nodes[0].available.iter().position(|x| *x).unwrap();
            e.nodes[0].probs[i] = row;
            assert!(validate_entry(&e).is_err(), "{fixture}");
        }
        let mut e = entry.clone();
        e.covered_paths.reverse();
        assert!(validate_entry(&e).is_err(), "{fixture}");
    }
}

// --- plan 4 task 8: the real materialized fixture ----------------------------------------------------------------

/// The committed production tree plan 4 Task 8 froze (`crates/engine/tests/golden/cache_scale.json`, `"entry"`: the
/// engine materializer's complete `flop_fast_v1` list at P = 100, stacks 500), wrapped in a valid entry the way Task 8's
/// `CacheRig` builds one: board Kh7d2c, public AA / KK ranges blocked by the board and canonicalized with it, every
/// flop node exported with the uniform legal probabilities and `EV(a) = 10 * n + a` chips (fold exactly 0), 5% rake
/// capped at 5 chips. The tree signature is the engine's to compute and `validate_entry` does not re-derive it, so the
/// template id stands in for it here. A committed artifact: a missing golden fails (standing ruling (e)).
fn real_entry() -> CacheEntry {
    use cache::key::{spr_bucket, KeyFields, Model, RakeKey, Rational};
    use proto::{Action, MenuSize, PlayerMenus, SideMenu, Street};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../engine/tests/golden/cache_scale.json");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("the committed fixture {} is missing: {e}", path.display()));
    let golden: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let materialized: Vec<proto::MaterializedNode> = serde_json::from_value(golden["entry"].clone()).unwrap();

    let board = ["Kh", "7d", "2c"].map(|c| proto::Card::parse(c).unwrap()).to_vec();
    let mut ranges = [core_ranges::parse_range("AA").unwrap(), core_ranges::parse_range("KK").unwrap()];
    for r in &mut ranges {
        core_ranges::block_public(r, &board);
    }
    let (_, perm) = core_iso::canonicalize(&board, &[&ranges[0], &ranges[1]]);
    let mut canonical_board = board.iter().map(|c| core_iso::apply(&perm, *c)).collect::<Vec<_>>();
    canonical_board.sort_by_key(|c| c.0);
    let ranges = [core_iso::apply_range(&perm, &ranges[0]), core_iso::apply_range(&perm, &ranges[1])];

    let side = SideMenu { bet: vec![MenuSize::Pot(0.5)], raise: vec![MenuSize::Pot(2.5)] };
    let menus = [Street::Flop, Street::Turn, Street::River]
        .into_iter()
        .map(|s| (s, PlayerMenus { oop: side.clone(), ip: side.clone(), donk: if s == Street::Flop { None } else { Some(vec![]) } }))
        .collect();
    let tree = proto::EffectiveTree {
        rules_version: 3,
        template_id: "flop_fast_v1".into(),
        root_street: Street::Flop,
        menus,
        add_allin_threshold: 1.0,
        force_allin_threshold: 0.15,
        merging_threshold: 0.0,
        wager_cap: 3,
        inserted: vec![],
        materialized,
    };
    let nodes = tree
        .materialized
        .iter()
        .filter(|m| m.street == Street::Flop)
        .enumerate()
        .map(|(n, m)| {
            let range = &ranges[if m.actor == "oop" { 0 } else { 1 }];
            let width = m.actions.len();
            let available = range.0.iter().map(|w| *w > 0.0).collect::<Vec<_>>();
            proto::worker::NodeStrategy {
                path: cache::entry::chip_path(&tree.materialized, &m.path).unwrap(),
                actor: m.actor.clone(),
                actions: m.actions.clone(),
                probs: available.iter().map(|a| if *a { vec![1.0 / width as f32; width] } else { vec![0.0; width] }).collect(),
                ev_chips: available
                    .iter()
                    .map(|a| m.actions.iter().enumerate().map(|(i, x)| if !*a || *x == Action::Fold { 0.0 } else { (10 * n + i) as f32 }).collect())
                    .collect(),
                available,
            }
        })
        .collect::<Vec<_>>();
    let solution = proto::worker::StreetSolution {
        covered_paths: nodes.iter().map(|n| n.path.clone()).collect(),
        nodes,
        requested: 0,
        exploitability_chips: 0.4,
        iterations: 1000,
        memory_bytes: 1 << 20,
        mode: "f32".into(),
        locks_applied: 0,
        export: "street".into(),
    };
    let nodes = cache::entry::normalize(&solution, &tree, 100).unwrap();
    let fractions = tree
        .materialized
        .iter()
        .map(|m| m.actions.iter().map(|a| cache::lookup::action_to(a).map(|to| Rational::new(to as u64, 100).unwrap())).collect())
        .collect();
    let spr = Rational::new(500, 100).unwrap();
    CacheEntry {
        key: KeyFields {
            schema_version: 3,
            solver_commit: proto::worker::SOLVER_COMMIT.into(),
            adapter_version: proto::worker::ADAPTER_VERSION,
            rules_version: 3,
            canonical_board,
            root_street: Street::Flop,
            spr_bucket: spr_bucket(spr),
            tree_signature: "flop_fast_v1".into(),
            rake: RakeKey::new(0.05, Rational::new(5000, 100_000).unwrap(), 1).unwrap(),
            range_hash_oop: core_ranges::hash_scaled(&ranges[0]),
            range_hash_ip: core_ranges::hash_scaled(&ranges[1]),
            model: Model::Baseline,
        },
        source: cache::entry::SourceInputs {
            pot: 100,
            stack_oop: 500,
            stack_ip: 500,
            spr,
            bb_chips: 2,
            quantum_over_p: Rational::new(1, 100).unwrap(),
            cap_mchips: 5000,
            ranges,
        },
        tree,
        fractions,
        covered_paths: nodes.iter().map(|n| n.path.clone()).collect(),
        nodes,
        exploitability_over_P: 0.004,
        target_bp: 50,
        iterations: 1000,
        elapsed_ms: 100,
        memory_bytes: 1 << 20,
        mode: "f32".into(),
        locks_applied: 0,
        export: "street".into(),
        reasons: vec![],
        created: 1,
        last_hit: 1,
    }
}

/// The fixtures Task 5's corruption mutations run on: Task 2's hand-built skeleton and the real materialized tree.
fn fixtures() -> Vec<(&'static str, CacheEntry)> {
    vec![("support::entry", support::entry()), ("real flop_fast_v1 100/500", real_entry())]
}

/// The real fixture is what it claims to be: the complete three-street production tree (the golden's 182 nodes, flop
/// root Check / Bet(50)), valid, and stored and read back unchanged through the real cell codec.
#[test]
fn the_real_materialized_fixture_is_valid_and_round_trips() {
    let e = real_entry();
    assert!(validate_entry(&e).is_ok());
    assert_eq!(e.tree.materialized.len(), 182);
    assert_eq!(e.tree.materialized[0].actions, vec![proto::Action::Check, proto::Action::Bet { to: 50 }]);
    assert!([proto::Street::Turn, proto::Street::River].iter().all(|s| e.tree.materialized.iter().any(|m| m.street == *s)));
    assert_eq!(e.nodes.len(), e.tree.materialized.iter().filter(|m| m.street == proto::Street::Flop).count());
    let dir = TempDir::new("real-fixture");
    let path = storage::entry_path(dir.path(), e.key.digest());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, storage::encode(&Cell { entries: vec![e.clone()] }).unwrap()).unwrap();
    let back = storage::read_cell(&path).expect("a valid real cell reads back");
    assert!(path.exists());
    assert_eq!(quota_digest(&back.entries[0]), quota_digest(&e));
}

fn quota_digest(e: &CacheEntry) -> Vec<u8> {
    cache::quota::entry_digest(e)
}

// --- review R1: lossless persistence -------------------------------------------------------------

/// A source range with negative-zero weights validates, encodes, and reads back bit-exactly --
/// unlike the brief's original JSON-metadata design, whose human-readable `Range1326` decoder
/// normalizes `-0.0` to `+0.0` (`crates/proto/src/range.rs`) and would desync the stored hash on
/// read, deleting a valid entry as if it were corrupt. `read_cell` must not delete this file.
#[test]
fn encode_decode_preserves_negative_zero_range_bits_and_read_cell_keeps_the_file() {
    let mut e = support::entry();
    for range in &mut e.source.ranges {
        for w in range.0.iter_mut() {
            if *w == 0.0 {
                *w = -0.0;
            }
        }
    }
    e.key.range_hash_oop = core_ranges::hash_scaled(&e.source.ranges[0]);
    e.key.range_hash_ip = core_ranges::hash_scaled(&e.source.ranges[1]);
    assert!(validate_entry(&e).is_ok(), "the modified-but-internally-consistent -0.0 entry must validate before any round trip");
    for range in &e.source.ranges {
        assert!(range.0.iter().any(|w| w.is_sign_negative() && *w == 0.0), "fixture must contain a genuine -0.0 to be a meaningful regression");
    }

    let dir = TempDir::new("signed-zero");
    let path = storage::entry_path(dir.path(), e.key.digest());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let bytes = storage::encode(&Cell { entries: vec![e.clone()] }).unwrap();
    std::fs::write(&path, &bytes).unwrap();

    let back = storage::read_cell(&path).expect("a lossless round trip must not be deleted as corrupt (review R1)");
    assert!(path.exists(), "a successful read must not delete the file");
    for (range, back_range) in e.source.ranges.iter().zip(back.entries[0].source.ranges.iter()) {
        assert!(
            range.0.iter().zip(back_range.0.iter()).all(|(a, b)| a.to_bits() == b.to_bits()),
            "every range weight's bit pattern, including sign, must survive the on-disk round trip"
        );
    }
    assert_eq!(core_ranges::hash_scaled(&back.entries[0].source.ranges[0]), back.entries[0].key.range_hash_oop);
    assert_eq!(core_ranges::hash_scaled(&back.entries[0].source.ranges[1]), back.entries[0].key.range_hash_ip);
    assert!(validate_entry(&back.entries[0]).is_ok());
}

/// The counterpart to the above: a range bit pattern that *disagrees* with its own stored hash
/// (rather than one that is internally self-consistent) must be rejected outright by both
/// `validate_entry` and `encode` -- there is no normalization step anywhere in this bincode-only
/// pipeline that could paper over the disagreement into false validity.
#[test]
fn encode_rejects_a_range_whose_negative_zero_bit_disagrees_with_its_stored_hash() {
    let mut e = support::entry();
    let i = e.source.ranges[0].0.iter().position(|w| *w == 0.0).unwrap();
    e.source.ranges[0].0[i] = -0.0;
    assert!(validate_entry(&e).is_err(), "validate_entry itself must already reject a hash/bits disagreement in memory");
    assert!(storage::encode(&Cell { entries: vec![e] }).is_err());
}

// --- review R2: preflight before allocation --------------------------------------------------

/// review R2: a single oversized key field (the report's own reproduction: an oversized
/// `tree_signature` string) must be caught by the explicit `DECODED_MAX` preflight check, not
/// merely error out some other way (e.g. an allocation failure) after the fact.
#[test]
fn encode_rejects_an_oversized_key_field_via_the_explicit_preflight_check() {
    let mut e = support::entry();
    e.key.tree_signature = "x".repeat((DECODED_MAX + 4096) as usize);
    match storage::encode(&Cell { entries: vec![e] }) {
        Err(cache::CacheError::Invalid(msg)) => assert_eq!(msg, "decoded cell payload exceeds DECODED_MAX"),
        other => panic!("expected the explicit DECODED_MAX rejection, got {other:?}"),
    }
}

/// review R2's literal request: two entries that each *individually* fit within `DECODED_MAX`,
/// but whose combined size does not, must still be rejected -- proving the budget is checked
/// against the whole cell, not approved per entry and only discovered to be over budget once
/// both are already fully materialized.
#[test]
fn encode_rejects_a_two_entry_cell_whose_combined_size_exceeds_decoded_max_though_each_individually_fits() {
    let big = "y".repeat((DECODED_MAX / 2 + 32 * 1024 * 1024) as usize); // ~160 MiB each
    let mut a = support::entry();
    a.key.tree_signature = big.clone();
    let mut b = support::entry();
    b.key.tree_signature = big;
    assert_eq!(a.key, b.key, "both entries must share one key for this to be a legal two-entry cell in the first place");
    assert!(validate_entry(&a).is_ok());
    assert!(validate_entry(&b).is_ok());

    let single = bincode::DefaultOptions::new().with_fixint_encoding().with_little_endian().serialized_size(&a).unwrap();
    assert!(single < DECODED_MAX, "each entry alone ({single} bytes) must fit within DECODED_MAX for this test to demonstrate the combined-only check");

    match storage::encode(&Cell { entries: vec![a, b] }) {
        Err(cache::CacheError::Invalid(msg)) => assert_eq!(msg, "decoded cell payload exceeds DECODED_MAX"),
        other => panic!("expected the explicit DECODED_MAX rejection, got {other:?}"),
    }
}

// --- decode: bounded header checks precede any bincode/zstd call -------------------------------

#[test]
fn bounded_header_rejects_oversized_lengths_before_decode() {
    let mut bytes = vec![0_u8; 56];
    bytes[..8].copy_from_slice(b"PAI3\x03\x00\x03\x00");
    bytes[8..16].copy_from_slice(&(cache::storage::COMPRESSED_MAX + 1).to_le_bytes());
    assert!(cache::storage::decode(&bytes).is_err());
}

/// review R2's "related test weakness" on the test above: it supplies no compressed body at
/// all, so the header/body length-*consistency* check alone would also reject it even if the
/// explicit `compressed_len > COMPRESSED_MAX` comparison were removed. This test makes the
/// header and body lengths agree (a real `COMPRESSED_MAX + 1`-byte body), so only the explicit
/// bound can catch it.
#[test]
fn decode_rejects_compressed_length_over_max_even_when_header_and_body_lengths_agree() {
    let filler = vec![0_u8; (COMPRESSED_MAX + 1) as usize];
    let mut bytes = Vec::with_capacity(56 + filler.len());
    bytes.extend_from_slice(b"PAI3");
    bytes.extend_from_slice(&3_u16.to_le_bytes());
    bytes.extend_from_slice(&3_u16.to_le_bytes());
    bytes.extend_from_slice(&(filler.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    bytes.extend_from_slice(&[0_u8; 32]);
    bytes.extend_from_slice(&filler);
    assert_eq!(bytes.len() as u64, 56 + COMPRESSED_MAX + 1, "header and body lengths must agree for this test to isolate the explicit bound");
    assert!(storage::decode(&bytes).is_err());
}

#[test]
fn decode_rejects_bad_magic_or_version() {
    let e = support::entry();
    let bytes = storage::encode(&Cell { entries: vec![e] }).unwrap();
    let mut bad_magic = bytes.clone();
    bad_magic[0] = b'X';
    assert!(storage::decode(&bad_magic).is_err());
    let mut bad_version = bytes;
    bad_version[4] = 4; // schema_version 4, never written by this codec
    assert!(storage::decode(&bad_version).is_err());
}

#[test]
fn decode_rejects_truncated_header() {
    let e = support::entry();
    let bytes = storage::encode(&Cell { entries: vec![e] }).unwrap();
    assert!(bytes.len() > 40, "an encoded cell must be longer than the truncated prefix below");
    assert!(storage::decode(&bytes[..40]).is_err());
}

/// A single flipped byte inside the compressed region must be caught -- either zstd refuses the
/// corrupted frame outright, or (if it happens to still decompress) the decoded-payload sha256
/// in the header no longer agrees with what was actually decompressed.
#[test]
fn decode_rejects_a_flipped_compressed_byte() {
    let e = support::entry();
    let mut bytes = storage::encode(&Cell { entries: vec![e] }).unwrap();
    assert!(bytes.len() > 56, "an encoded cell must carry a nonempty compressed payload");
    let last = bytes.len() - 1;
    bytes[last] ^= 0xFF;
    assert!(storage::decode(&bytes).is_err());
}

/// A "zip bomb": a real, genuinely-larger-than-the-cap zstd stream (twice `DECODED_MAX`, from an
/// all-zero source, so building and compressing it stays fast). Review R2/R4's "not timing"
/// instruction: rather than inferring boundedness from how fast this test runs, it directly
/// measures the *count* of bytes the same capped-reader pattern `decode` uses internally
/// actually returns (`capped_decompress`, mirroring `decode`'s own `window_log_max`/`take`
/// construction) -- proving the cap truncates a much larger stream at exactly `DECODED_MAX + 1`
/// bytes -- and separately confirms `decode` itself rejects the same bomb end to end.
#[test]
fn zstd_bomb_is_capped_by_count_and_rejected_end_to_end() {
    let real_len = DECODED_MAX * 2 + 1;
    let bomb = vec![0_u8; real_len as usize];
    let compressed = zstd::stream::encode_all(bomb.as_slice(), 3).unwrap();
    drop(bomb);
    assert!((compressed.len() as u64) < COMPRESSED_MAX, "an all-zero source must compress far below COMPRESSED_MAX for this test to stay meaningful");

    let capped = capped_decompress(&compressed);
    assert_eq!(capped.len() as u64, DECODED_MAX + 1, "the capped reader must truncate a {real_len}-byte stream at DECODED_MAX + 1 bytes, by count, not by how long this took");
    drop(capped);

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"PAI3");
    bytes.extend_from_slice(&3_u16.to_le_bytes());
    bytes.extend_from_slice(&3_u16.to_le_bytes());
    bytes.extend_from_slice(&(compressed.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&DECODED_MAX.to_le_bytes()); // lies low: the real stream is far larger
    bytes.extend_from_slice(&[0_u8; 32]); // sha256 is unreachable: the length check fails first
    bytes.extend_from_slice(&compressed);
    assert!(storage::decode(&bytes).is_err());
}

// --- encode/decode round trip -------------------------------------------------------------------

#[test]
fn encode_decode_round_trips_a_single_entry_cell() {
    let e = support::entry();
    let bytes = storage::encode(&Cell { entries: vec![e.clone()] }).unwrap();
    let back = storage::decode(&bytes).unwrap();
    assert_eq!(back.entries.len(), 1);
    assert_eq!(back.entries[0].key, e.key);
    assert_eq!(back.entries[0].nodes.len(), e.nodes.len());
    for (a, b) in e.nodes.iter().zip(back.entries[0].nodes.iter()) {
        assert_eq!(a.path, b.path);
        assert_eq!(a.actor, b.actor);
        assert_eq!(a.probs, b.probs);
        assert_eq!(a.ev_over_P, b.ev_over_P);
        assert_eq!(a.available, b.available);
    }
    assert!(validate_entry(&back.entries[0]).is_ok());
}

#[test]
fn encode_decode_round_trips_a_two_entry_cell() {
    let e = support::entry();
    let bytes = storage::encode(&Cell { entries: vec![e.clone(), e.clone()] }).unwrap();
    let back = storage::decode(&bytes).unwrap();
    assert_eq!(back.entries.len(), 2);
    for got in &back.entries {
        assert_eq!(got.key, e.key);
        assert!(validate_entry(got).is_ok());
    }
}

// --- entry_path / read_cell against the filesystem ----------------------------------------------

#[test]
fn entry_path_shards_by_the_keys_first_two_hex_characters() {
    let mut key = [0_u8; 32];
    key[0] = 0xab;
    key[1] = 0xcd;
    let dir = std::path::Path::new("v3");
    let path = storage::entry_path(dir, key);
    let hex = key.iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(path, dir.join(&hex[..2]).join(format!("{hex}.bin")));
    assert!(path.starts_with(dir.join("ab")), "the shard directory must be the key's first two hex characters");
    assert_eq!(path.file_name().unwrap().to_str().unwrap(), format!("{hex}.bin"));
}

#[test]
fn read_cell_reads_back_a_validly_written_cell_and_leaves_it_in_place() {
    let e = support::entry();
    let bytes = storage::encode(&Cell { entries: vec![e.clone()] }).unwrap();
    let dir = TempDir::new("read");
    let path = storage::entry_path(dir.path(), e.key.digest());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &bytes).unwrap();

    let back = storage::read_cell(&path).expect("a validly written cell must be a hit");
    assert_eq!(back.entries.len(), 1);
    assert_eq!(back.entries[0].key, e.key);
    assert!(path.exists(), "a successful read must not delete the file");
}

// --- review R3: deletion race --------------------------------------------------------------------
//
// Fix round 2 (review N1): the controlled-interleaving regression that needs the hook
// (`read_cell_does_not_delete_a_replacement_published_between_read_and_cleanup`) moved into
// `crates/cache/src/storage.rs`'s own `#[cfg(test)] mod tests`, with every assertion unchanged --
// the hook it depends on is now `#[cfg(test)]` and non-public, so it is only reachable from that
// crate-internal unit test, never from this integration-test crate. The ordinary (non-racing)
// case below needs no hook at all, so it stays here using only `read_cell`.

/// The ordinary (non-racing) case: when nothing else touches the path, the identity check
/// agrees and a genuinely corrupt file is still deleted exactly as before.
#[test]
fn read_cell_still_deletes_an_undisturbed_corrupt_file() {
    let dir = TempDir::new("no-race");
    let path = dir.path().join("cell.bin");
    std::fs::write(&path, b"not a cache header").unwrap();
    assert!(storage::read_cell(&path).is_none());
    assert!(!path.exists());
}

// --- review R4: the full corruption matrix, through read_cell -----------------------------------

/// Struct-level mutations (review R4): each of these stays in-domain per scalar value (only
/// cross-field/shape/identity/order invariants are broken), so each still reaches a real,
/// correctly checksummed file through `write_unchecked` -- and each must be a miss with the file
/// deleted when read back through the real `read_cell`, not merely rejected by `validate_entry`
/// in isolation.
#[test]
fn cache_storage_rejects_and_deletes_struct_level_corrupt_payloads() {
    let dir = TempDir::new("struct-corrupt");

    let cases: Vec<(&str, Box<dyn Fn(&mut CacheEntry)>)> = vec![
        ("row_sum_wrong_but_in_domain", Box::new(|e: &mut CacheEntry| {
            let i = e.nodes[0].available.iter().position(|x| *x).unwrap();
            e.nodes[0].probs[i] = vec![0.2, 0.2];
        })),
        ("wrong_1326_shape", Box::new(|e: &mut CacheEntry| {
            e.nodes[0].probs.pop();
        })),
        ("nonzero_unavailable_prob", Box::new(|e: &mut CacheEntry| {
            let j = e.nodes[0].available.iter().position(|a| !a).unwrap();
            e.nodes[0].probs[j] = vec![1.0, 0.0];
        })),
        ("nonzero_unavailable_ev", Box::new(|e: &mut CacheEntry| {
            let j = e.nodes[0].available.iter().position(|a| !a).unwrap();
            e.nodes[0].ev_over_P[j] = vec![0.1, 0.0];
        })),
        ("covered_paths_reordered", Box::new(|e: &mut CacheEntry| {
            e.covered_paths.reverse();
        })),
        ("invalid_node_path", Box::new(|e: &mut CacheEntry| {
            e.nodes[0].path = vec![255];
            e.covered_paths[0] = vec![255];
        })),
        ("wrong_actor", Box::new(|e: &mut CacheEntry| {
            e.nodes[0].actor = "not-a-real-actor".into();
        })),
        ("duplicate_path", Box::new(|e: &mut CacheEntry| {
            let path = e.nodes[0].path.clone();
            let actor = e.nodes[0].actor.clone();
            e.nodes[1].path = path.clone();
            e.nodes[1].actor = actor;
            e.covered_paths[1] = path;
        })),
        ("wrong_schema_version", Box::new(|e: &mut CacheEntry| {
            e.key.schema_version = 2;
        })),
        ("wrong_rules_version", Box::new(|e: &mut CacheEntry| {
            e.key.rules_version = 2;
        })),
    ];

    // Plan 4 Task 8: on both fixtures, Task 2's skeleton and the real materialized tree.
    for (k, (fixture, entry)) in fixtures().into_iter().enumerate() {
        for (name, mutate) in &cases {
            let mut e = entry.clone();
            mutate(&mut e);
            assert!(validate_entry(&e).is_err(), "{fixture}: {name} must actually be invalid, or this case tests nothing");
            let bytes = write_unchecked(&[e]);
            assert_miss_and_deleted(&bytes, dir.path(), &format!("{k}-{name}.bin"));
        }
    }
}

/// Byte-level corruptions (review R4): `-0.1`/`1.1` probabilities, a nonfinite EV, and a
/// nonfinite `exploitability_over_P` cannot reach a byte stream through any checked serializer
/// (`CachedNode`'s own `Serialize` enforces domain/finiteness on every format), so each is
/// produced by starting from a real, valid `encode()` output, patching a distinctive sentinel
/// value's native-bincode bytes directly in the decompressed payload, and reframing (recompute
/// compression, lengths, and sha256) -- then read back through the real `read_cell`, asserting
/// miss and deletion.
#[test]
fn cache_storage_rejects_and_deletes_byte_level_corrupt_payloads() {
    let dir = TempDir::new("byte-corrupt");

    // Plan 4 Task 8: on both fixtures, Task 2's skeleton and the real materialized tree.
    for (k, (fixture, entry)) in fixtures().into_iter().enumerate() {
        // Out-of-domain probabilities (-0.1, 1.1): patch one value of an otherwise-valid, row-summing
        // pair so only the *patched* value itself is out of domain.
        for (name, sentinel, corrupt) in [("prob_1_1", 0.411_337_f32, 1.1_f32), ("prob_neg_0_1", 0.522_337_f32, -0.1_f32)] {
            let mut e = entry.clone();
            let i = e.nodes[0].available.iter().position(|x| *x).unwrap();
            e.nodes[0].probs[i] = vec![sentinel, 1.0 - sentinel];
            let bytes = storage::encode(&Cell { entries: vec![e] }).unwrap();
            let mut payload = decompressed_payload_of(&bytes);
            let needle = sentinel.to_le_bytes();
            let occurrences = payload.windows(4).filter(|w| *w == needle).count();
            assert_eq!(occurrences, 1, "{fixture}: sentinel {sentinel} must appear exactly once in the decoded payload");
            let pos = payload.windows(4).position(|w| w == needle).unwrap();
            payload[pos..pos + 4].copy_from_slice(&corrupt.to_le_bytes());
            assert_miss_and_deleted(&reframe(&payload), dir.path(), &format!("{k}-{name}.bin"));
        }

        // Nonfinite EV: patch a populated combo's non-fold EV to NaN.
        {
            let sentinel = 24_681.359_f32;
            let mut e = entry.clone();
            let i = e.nodes[0].available.iter().position(|x| *x).unwrap();
            e.nodes[0].ev_over_P[i][1] = sentinel;
            let bytes = storage::encode(&Cell { entries: vec![e] }).unwrap();
            let mut payload = decompressed_payload_of(&bytes);
            let needle = sentinel.to_le_bytes();
            let occurrences = payload.windows(4).filter(|w| *w == needle).count();
            assert_eq!(occurrences, 1, "{fixture}: sentinel must appear exactly once in the decoded payload");
            let pos = payload.windows(4).position(|w| w == needle).unwrap();
            payload[pos..pos + 4].copy_from_slice(&f32::NAN.to_le_bytes());
            assert_miss_and_deleted(&reframe(&payload), dir.path(), &format!("{k}-nonfinite_ev.bin"));
        }

        // Nonfinite exploitability_over_P (f64, top-level CacheEntry field).
        {
            let sentinel = 0.041_233_7_f64;
            let mut e = entry.clone();
            e.exploitability_over_P = sentinel;
            let bytes = storage::encode(&Cell { entries: vec![e] }).unwrap();
            let mut payload = decompressed_payload_of(&bytes);
            let needle = sentinel.to_le_bytes();
            let occurrences = payload.windows(8).filter(|w| *w == needle).count();
            assert_eq!(occurrences, 1, "{fixture}: sentinel must appear exactly once in the decoded payload");
            let pos = payload.windows(8).position(|w| w == needle).unwrap();
            payload[pos..pos + 8].copy_from_slice(&f64::NAN.to_le_bytes());
            assert_miss_and_deleted(&reframe(&payload), dir.path(), &format!("{k}-nonfinite_exploitability.bin"));
        }
    }
}
