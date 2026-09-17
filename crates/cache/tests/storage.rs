//! Bounded binary storage for the on-disk street-solution cache (spec section 10.4; plan 4 task
//! 5): `Cell`, `encode`/`decode`, `entry_path`, `read_cell`. A cell holds one or two `CacheEntry`
//! values sharing the same structural key digest; on disk it is a fixed 56-byte header (magic,
//! schema/proto version, compressed/decoded length, decoded-payload sha256) followed by a
//! zstd-compressed bincode payload. Every bound (cell entry count, key digest agreement, header
//! size fields, decoded/compressed byte ceilings) is checked before the corresponding allocation
//! or decode call, and any read/decode failure is a cache miss with best-effort deletion of the
//! offending file (CLAUDE.md section 6 "Read/decode error: miss, best-effort delete").

mod support;

use bincode::Options;
use cache::entry::{validate_entry, CacheEntry, CachedNode};
use cache::storage::{self, Cell, COMPRESSED_MAX, DECODED_MAX};
use sha2::{Digest, Sha256};

#[test]
fn cache_corrupt_entry_deleted() {
    let dir = std::env::temp_dir().join(format!("pokerai-cache-corrupt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bad.bin");
    std::fs::write(&path, b"not a cache header").unwrap();
    assert!(cache::storage::read_cell(&path).is_none());
    assert!(!path.exists());
    // A directory cannot be removed as a file: deletion failure is still a miss.
    assert!(cache::storage::read_cell(&dir).is_none());
    std::fs::remove_dir(&dir).unwrap();
}

// --- encode: cell-level invariants ------------------------------------------------------------

/// review m4: entries that disagree on their own structural key digest have no business sharing
/// a cell -- `encode` must reject this before ever validating or writing either entry.
#[test]
fn cell_key_digest_mismatch_rejected_by_encode() {
    let a = support::entry();
    let mut b = support::entry();
    b.key.spr_bucket += 1; // changes b.key.digest() without changing b.source, a's twin otherwise
    assert_ne!(a.key.digest(), b.key.digest());
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
/// reach a byte stream (`CachedNode`'s own `Serialize` enforces the `[0, 1]` domain on every
/// format, including bincode; see `crates/cache/src/entry.rs`'s `native_checked`), so these are
/// exercised against `validate_entry` in memory, not through `encode`/`decode`.
#[test]
fn cache_payload_validated() {
    for row in [vec![0.2, 0.2], vec![-0.1, 1.1], vec![1.1, -0.1]] {
        let mut e = support::entry();
        let i = e.nodes[0].available.iter().position(|x| *x).unwrap();
        e.nodes[0].probs[i] = row;
        assert!(validate_entry(&e).is_err());
    }
    let mut e = support::entry();
    e.covered_paths.reverse();
    assert!(validate_entry(&e).is_err());
}

// --- decode: bounded header checks precede any bincode/zstd call -------------------------------

#[test]
fn bounded_header_rejects_oversized_lengths_before_decode() {
    let mut bytes = vec![0_u8; 56];
    bytes[..8].copy_from_slice(b"PAI3\x03\x00\x03\x00");
    bytes[8..16].copy_from_slice(&(cache::storage::COMPRESSED_MAX + 1).to_le_bytes());
    assert!(cache::storage::decode(&bytes).is_err());
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

/// A "zip bomb": the header declares a decoded length within `DECODED_MAX`, but the actual zstd
/// stream decompresses to one byte more than `DECODED_MAX`. `decode` must reject this via its
/// length check rather than by allocating past `DECODED_MAX` to discover the mismatch -- the
/// capped `take(DECODED_MAX + 1)` reader is what keeps this test itself fast and bounded.
#[test]
fn decode_rejects_a_zstd_bomb_exceeding_decoded_max() {
    let real_decoded_len = DECODED_MAX + 1;
    let bomb = vec![0_u8; real_decoded_len as usize];
    let compressed = zstd::stream::encode_all(bomb.as_slice(), 3).unwrap();
    assert!((compressed.len() as u64) < COMPRESSED_MAX, "an all-zero source must compress far below COMPRESSED_MAX for this test to stay meaningful");

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"PAI3");
    bytes.extend_from_slice(&3_u16.to_le_bytes());
    bytes.extend_from_slice(&3_u16.to_le_bytes());
    bytes.extend_from_slice(&(compressed.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&DECODED_MAX.to_le_bytes()); // lies low: the real stream is one byte over
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
    assert_eq!(back.entries[0].key.digest(), e.key.digest());
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
        assert_eq!(got.key.digest(), e.key.digest());
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
    let dir = std::env::temp_dir().join(format!("pokerai-cache-read-{}", std::process::id()));
    let path = storage::entry_path(&dir, e.key.digest());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &bytes).unwrap();

    let back = storage::read_cell(&path).expect("a validly written cell must be a hit");
    assert_eq!(back.entries.len(), 1);
    assert_eq!(back.entries[0].key.digest(), e.key.digest());
    assert!(path.exists(), "a successful read must not delete the file");

    std::fs::remove_dir_all(&dir).unwrap();
}

// --- storage-level content validation: a corrupt-but-checksummed cell is still rejected --------

/// Mirrors `cache::storage::encode`'s on-disk framing exactly, except it never calls
/// `cache::entry::validate_entry` -- used only to get a structurally well-formed, correctly
/// checksummed file onto disk whose *content* is invalid, so a subsequent `read_cell` miss can be
/// attributed to content validation, not just the checksum (brief step 5). `CachedNode` still
/// goes through its own checked `Serialize`, so this cannot (and is not asked to) carry a bare
/// out-of-domain scalar -- those are exercised directly against `validate_entry` in
/// `cache_payload_validated` above instead. Not exported from `cache` itself (brief step 5: "Do
/// not export an unchecked writer from cache").
fn write_unchecked(entries: &[CacheEntry]) -> Vec<u8> {
    let disk: Vec<(Vec<u8>, Vec<CachedNode>)> = entries
        .iter()
        .map(|e| {
            let mut metadata = e.clone();
            metadata.nodes.clear();
            (serde_json::to_vec(&metadata).expect("mutated-but-in-domain metadata must still be JSON-serializable"), e.nodes.clone())
        })
        .collect();
    let options = || bincode::DefaultOptions::new().with_fixint_encoding().with_little_endian().with_limit(DECODED_MAX).reject_trailing_bytes();
    let payload = options().serialize(&disk).expect("mutated-but-in-domain entries must still bincode-serialize");
    let compressed = zstd::stream::encode_all(payload.as_slice(), 3).unwrap();
    let mut out = Vec::with_capacity(56 + compressed.len());
    out.extend_from_slice(b"PAI3");
    out.extend_from_slice(&3_u16.to_le_bytes());
    out.extend_from_slice(&3_u16.to_le_bytes());
    out.extend_from_slice(&(compressed.len() as u64).to_le_bytes());
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(&Sha256::digest(&payload));
    out.extend_from_slice(&compressed);
    out
}

fn assert_miss_and_deleted(bytes: &[u8], dir: &std::path::Path, name: &str) {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    assert!(storage::read_cell(&path).is_none(), "{name} must be a cache miss");
    assert!(!path.exists(), "{name} must be deleted after a failed read");
}

/// The brief's longer parameter-case list (shape, availability-vs-content, path/actor/order
/// disagreement, version numbers) written through the unchecked framing above and read back
/// through the real `read_cell` -- every one of these must be a miss with the file deleted, and
/// the rejection must come from `decode`'s re-run of `validate_entry`, not from a checksum
/// mismatch (the checksum here is always freshly and correctly computed over the corrupted
/// payload).
#[test]
fn cache_storage_rejects_and_deletes_corrupt_payload_content_not_just_checksum() {
    let dir = std::env::temp_dir().join(format!("pokerai-cache-payload-storage-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let cases: Vec<(&str, Box<dyn Fn(&mut CacheEntry)>)> = vec![
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

    for (name, mutate) in cases {
        let mut e = support::entry();
        mutate(&mut e);
        // Every mutation above stays in-domain per value (only cross-field/shape/identity
        // invariants are broken), so it must still reach the filesystem with a correct checksum.
        assert!(validate_entry(&e).is_err(), "{name} must actually be invalid, or this case tests nothing");
        let bytes = write_unchecked(&[e]);
        assert_miss_and_deleted(&bytes, &dir, &format!("{name}.bin"));
    }

    std::fs::remove_dir_all(&dir).unwrap();
}
