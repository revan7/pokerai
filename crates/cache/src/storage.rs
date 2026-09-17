//! Bounded, checksummed binary storage for the on-disk street-solution cache (spec section 10.4):
//! a `Cell` holds the one or two `CacheEntry` values kept per structural key, `encode`/`decode`
//! convert a `Cell` to and from its on-disk bytes, `entry_path` derives the sharded file path for
//! a key digest, and `read_cell` is the read side that turns any decode failure into a cache miss
//! with best-effort deletion of the offending file (CLAUDE.md section 6: "Read/decode error:
//! miss, best-effort delete").
//!
//! On-disk layout is a fixed 56-byte header followed by a zstd-compressed bincode payload:
//! magic `PAI3` (4 bytes), `schema_version` (u16 LE), `proto_version` (u16 LE), compressed length
//! (u64 LE), decoded length (u64 LE), sha256 of the decoded (post-decompression, pre-bincode-
//! decode) payload (32 bytes). Every length in the header is checked against `COMPRESSED_MAX` /
//! `DECODED_MAX` (and, on read, against the file's own actual length) before any allocation or
//! bincode/zstd call runs against it, so a hostile or truncated header cannot force an oversized
//! allocation or an unbounded decompression ("zip bomb") -- the bound is checked, never assumed
//! from the data itself.
//!
//! The payload is `Vec<CacheEntry>` (one or two entries), bincode-encoded whole -- no separate
//! JSON layer (review round 1, R1; see "Fix round 1" below). `encode` re-runs
//! `crate::entry::validate_entry` on every entry before it is ever written, and `decode` re-runs
//! it on every entry it reads back -- storage never persists or discloses an entry that fails
//! the structural/numeric invariants `validate_entry` already enforces elsewhere, and a corrupt
//! payload that happens to keep a valid checksum is still caught by that content check, not only
//! by the sha256 (Interfaces: "Boundaries are per cell, including its two entries").
//!
//! Cells whose entries do not share one structural key do not share a cell: `encode` and
//! `decode` both reject that (a writer invariant too, review m4: two entries that disagree on
//! their own key have no business being written into, or read back out of, the same cell).
//!
//! ## Fix round 1 (review `task-5-review.md`, R1-R6)
//!
//! - **R1 (lossless persistence):** the brief's original design split each entry into a
//!   JSON-encoded `metadata` blob (everything but `nodes`) plus a separately bincode-encoded
//!   `nodes` list, so that JSON's own wide (f64) numeric validation would run on decode. That
//!   JSON layer is not lossless: `Range1326`'s human-readable `Deserialize`
//!   (`crates/proto/src/range.rs`) normalizes every `-0.0` weight to `+0.0`, while
//!   `core_ranges::hash_scaled` and `core_iso::canonicalize`'s tie-break both distinguish the two
//!   bit patterns (spec section 2) -- so a canonical entry with a negative-zero range weight
//!   could pass `validate_entry` and `encode`, then fail `decode`'s own re-run of
//!   `validate_entry` (hash mismatch) purely from lossy metadata persistence, not real
//!   corruption. Since a prior, independent follow-up already made every type reachable from
//!   `CacheEntry` (including `Range1326`, `Action`, `ApproxReason`, `MenuSize`) round-trip
//!   bit-exactly through bincode (`crates/proto/tests/bincode_wire.rs`,
//!   `crates/cache/tests/entry.rs`'s `cache_entry_round_trips_through_bincode_with_the_support_fixture`
//!   and `cache_entry_bincode_preserves_signed_zero_range_bits_after_binary_round_trip`), the fix
//!   (orchestrator ruling) is to persist the *whole* entry -- metadata and nodes together -- with
//!   the one bounded bincode configuration `nodes` already used, rather than splitting formats at
//!   all. This is a departure from the brief's literal `DiskEntry`/JSON-metadata sample: the
//!   brief predates the bincode-round-trip follow-up, and its JSON design is superseded by that
//!   ruling for this reason.
//! - **R2 (allocate only after the aggregate is accepted):** the removed JSON-metadata design
//!   also cloned each entry (to clear `nodes` for the metadata half) and separately cloned
//!   `nodes` again for the binary half, then bounded *each entry's own* JSON blob independently
//!   at `DECODED_MAX` before ever checking the combined cell size -- so two large entries could
//!   each allocate close to `DECODED_MAX` before the aggregate rejection ran. `encode` now builds
//!   a `Vec<&CacheEntry>` of borrowed references (no clone anywhere in this function) and calls
//!   `serialized_size` -- a bounded counting pass that sums byte-lengths without allocating an
//!   output buffer -- over the *whole* cell before calling `serialize` at all. The cell-agreement
//!   check no longer calls `KeyFields::digest()` either: that method's own uncapped
//!   `serde_json::to_vec` (`crates/cache/src/key.rs`) could itself be driven into an oversized
//!   allocation by an oversized `tree_signature` before any bound is checked, and `KeyFields`
//!   already derives `PartialEq`, so a direct structural comparison is both cheaper and exactly
//!   equivalent (two keys are digest-equal iff they are field-equal, since the digest is a
//!   deterministic hash of a fixed-schema canonical serialization).
//! - **R3 (deletion race):** `read_cell` opens the file, reads through that handle, and used to
//!   `remove_file(path)` unconditionally on any decode failure -- but a cache-writer could
//!   publish a valid replacement at that exact path in the window between this read and that
//!   unlink (the read handle is dropped once reading finishes, before decode even runs), and the
//!   pathname-based delete would then remove the *replacement*, not the corrupt bytes actually
//!   read. `read_cell` now captures `(len, last_write_time)` from the *opened handle's* own
//!   metadata before reading, and re-checks a *fresh*, path-based metadata read against that
//!   pair immediately before removing anything; a mismatch is treated the same as a deletion
//!   failure (still a miss, file left alone). This is deliberately not the NTFS file-index /
//!   volume-serial-number pair the review asked for: `std::os::windows::fs::MetadataExt::file_index`
//!   and `volume_serial_number` are still gated behind the unstable `windows_by_handle` feature
//!   (confirmed by direct compilation against this project's pinned `stable-x86_64-pc-windows-msvc`
//!   toolchain -- CLAUDE.md section 6 -- which rejected both with `error[E0658]`), so they are not
//!   usable here. `(len, last_write_time)` is a best-effort proxy, not a lock: a replacement that
//!   happened to land at the exact same size and the exact same 100ns-resolution last-write tick
//!   as the original would not be distinguished (astronomically unlikely for a real publish, but
//!   a real residual window), and there is a second, smaller window between the fresh check and
//!   the `remove_file` call itself. Making publication and cleanup mutually exclusive under one
//!   lock closes both windows properly and is Task 6's job (it owns `write_atomic` and the
//!   writer thread), not this read path's -- so no lock is introduced here.
//!
//! R4 (the storage-level corruption-case matrix) and R5 (test temp-directory hygiene) are test
//! changes only; see `crates/cache/tests/storage.rs`. R6 is a report correction only.

use crate::entry::CacheEntry;
use crate::CacheError;
use bincode::Options;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Ceiling on a cell's on-disk *compressed* byte length (spec section 10.4: "64 MiB compressed").
pub const COMPRESSED_MAX: u64 = 64 * 1024 * 1024;
/// Ceiling on a cell's *decoded* (post-decompression, pre-bincode-decode) byte length (spec
/// section 10.4: "256 MiB decoded").
pub const DECODED_MAX: u64 = 256 * 1024 * 1024;

const MAGIC: &[u8; 4] = b"PAI3";
const HEADER_LEN: u64 = 56;
/// Cache `schema_version` and `proto_version` (constraints.md: "cache `schema_version` 3;
/// ... `proto_version` 3") -- both pinned to the same value 3, but checked as the two distinct
/// header fields the spec names rather than folded into one.
const SCHEMA_VERSION: u16 = 3;
const PROTO_VERSION: u16 = 3;

/// The one or two `CacheEntry` values kept for a single structural key (spec section 10.4:
/// "replacement keeps at most two entries per cell"). `encode`/`decode` treat the whole cell,
/// including both entries, as one bounded unit -- there is no per-entry byte ceiling distinct
/// from the cell's own `COMPRESSED_MAX`/`DECODED_MAX`.
pub struct Cell {
    pub entries: Vec<CacheEntry>,
}

/// The bincode configuration the whole cell payload (`Vec<&CacheEntry>` on write, `Vec<CacheEntry>`
/// on read) is serialized and deserialized under: fixed-width (not varint) integers,
/// little-endian, capped at `DECODED_MAX`, and no trailing bytes tolerated after the last value.
/// `encode` and `decode` both call this rather than each building their own configuration, so the
/// two directions can never silently drift apart.
fn options() -> impl Options {
    bincode::DefaultOptions::new().with_fixint_encoding().with_little_endian().with_limit(DECODED_MAX).reject_trailing_bytes()
}

/// The exact aggregate bincode size `value` would need, with *no* limit attached. Used only for
/// the preflight measurement in `encode` (review R2): a bounded (`with_limit`) size count
/// aborts as soon as the running total exceeds the limit, returning `bincode::ErrorKind::SizeLimit`
/// rather than the true count -- which is fine for enforcing the bound, but loses the number this
/// module wants for its own clearer, cache-specific error message. Critically, *unbounded* does
/// not mean *unchecked-allocation*: `serialized_size` (bounded or not) walks the value and sums
/// byte-lengths without ever allocating an output buffer proportional to that sum, so measuring
/// the true size this way is still exactly the "bounded counting serializer... before allocating"
/// preflight the review asked for, not a second unbounded-allocation risk.
fn measure<T: ?Sized + serde::Serialize>(value: &T) -> Result<u64, CacheError> {
    Ok(bincode::DefaultOptions::new().with_fixint_encoding().with_little_endian().serialized_size(value)?)
}

/// The sharded on-disk path for a cell keyed by `key` (a `crate::key::KeyFields::digest()`),
/// rooted at `dir` (spec section 10.4: `<cache root>\v3\<key[0..2]>\<key>.bin`, `dir` already
/// being that `v3` root).
pub fn entry_path(dir: &Path, key: [u8; 32]) -> PathBuf {
    let hex = key.iter().map(|b| format!("{b:02x}")).collect::<String>();
    dir.join(&hex[..2]).join(format!("{hex}.bin"))
}

/// Encodes `cell` to its on-disk bytes: requires every entry to share the same structural key
/// (checked by direct `KeyFields` equality, review R2) and to pass `crate::entry::validate_entry`,
/// preflights the whole cell's bincode size with a bounded counting serializer over *borrowed*
/// entry references (no clone anywhere in this function, review R2) before allocating anything
/// proportional to it, zstd-compresses the resulting payload, checks the compressed result
/// against `COMPRESSED_MAX`, and prefixes the fixed 56-byte header (magic, schema/proto version,
/// compressed/decoded length, decoded-payload sha256).
///
/// # Errors
/// `CacheError::Invalid` if `cell` is empty or has more than two entries, if its entries disagree
/// on their key, if any entry fails `validate_entry`, or if the decoded or compressed payload
/// exceeds its bound. Propagates any bincode or I/O (zstd) failure.
pub fn encode(cell: &Cell) -> Result<Vec<u8>, CacheError> {
    if cell.entries.is_empty() || cell.entries.len() > 2 {
        return Err(CacheError::Invalid("cell entry count must be 1 or 2"));
    }
    // review R2: direct structural equality, not `KeyFields::digest()` -- see the module doc.
    if cell.entries.iter().any(|e| e.key != cell.entries[0].key) {
        return Err(CacheError::Invalid("cell entries disagree on key"));
    }
    for e in &cell.entries {
        crate::entry::validate_entry(e)?;
    }
    // review R2: a borrowed view, not an owned clone. `&CacheEntry` serializes byte-identically
    // to an owned `CacheEntry` (serde's blanket reference `Serialize` impl), so `decode`
    // reconstructs `Vec<CacheEntry>` unchanged from what this produces.
    let borrowed: Vec<&CacheEntry> = cell.entries.iter().collect();
    // review R2: measure the exact aggregate size a bounded counting serializer would need
    // *before* allocating any buffer for it (`measure`, above -- it counts without allocating a
    // buffer proportional to the count, bounded or not).
    let decoded_len = measure(&borrowed)?;
    if decoded_len > DECODED_MAX {
        return Err(CacheError::Invalid("decoded cell payload exceeds DECODED_MAX"));
    }
    // Only now does an allocation proportional to the (already-bounded) payload size happen.
    // `Options::serialize` repeats this same size-then-allocate ordering internally (bincode
    // 1.3.3's `internal::serialize` calls `serialized_size` before `Vec::with_capacity`), so this
    // is deliberate defense in depth with a clearer, cache-specific error message, not the only
    // thing standing between this function and an oversized allocation.
    let payload = options().serialize(&borrowed)?;
    let compressed = zstd::stream::encode_all(payload.as_slice(), 3)?;
    if compressed.len() as u64 > COMPRESSED_MAX {
        return Err(CacheError::Invalid("compressed cell payload exceeds COMPRESSED_MAX"));
    }
    let mut out = Vec::with_capacity(HEADER_LEN as usize + compressed.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    out.extend_from_slice(&PROTO_VERSION.to_le_bytes());
    out.extend_from_slice(&(compressed.len() as u64).to_le_bytes());
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(&Sha256::digest(&payload));
    out.extend_from_slice(&compressed);
    Ok(out)
}

/// Decodes `bytes` back to a `Cell`: checks the fixed 56-byte header (magic, schema/proto
/// version, both length fields against `COMPRESSED_MAX`/`DECODED_MAX`, and the actual input
/// length against the declared compressed length) *before* touching zstd or bincode, streams the
/// zstd decompression through a decoder capped (`window_log_max`) and read-limited so a "zip
/// bomb" cannot force an unbounded allocation, checks the decompressed length and sha256 against
/// the header, bincode-decodes the payload (bounded by the same `options()` `decode` uses),
/// re-validates every entry (`crate::entry::validate_entry`) and requires them to share one key.
///
/// # Errors
/// `CacheError::Invalid` (wrapped by the local `invalid` closure) for any header, length,
/// checksum, entry-count or key mismatch. Propagates any I/O (zstd) or bincode failure from a
/// malformed compressed stream or payload.
pub fn decode(bytes: &[u8]) -> Result<Cell, CacheError> {
    let invalid = || CacheError::Invalid("cache cell header or payload is invalid");
    let sv = SCHEMA_VERSION.to_le_bytes();
    let pv = PROTO_VERSION.to_le_bytes();
    let want_versions = [sv[0], sv[1], pv[0], pv[1]];
    if bytes.len() < HEADER_LEN as usize || &bytes[..4] != MAGIC || bytes[4..8] != want_versions {
        return Err(invalid());
    }
    let compressed_len = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
    let decoded_len = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
    if compressed_len > COMPRESSED_MAX || decoded_len > DECODED_MAX || bytes.len() as u64 != HEADER_LEN + compressed_len {
        return Err(invalid());
    }
    let mut reader = zstd::stream::read::Decoder::new(&bytes[HEADER_LEN as usize..])?;
    reader.window_log_max(28)?;
    let mut payload = Vec::new();
    reader.take(DECODED_MAX + 1).read_to_end(&mut payload)?;
    if payload.len() as u64 != decoded_len || Sha256::digest(&payload).as_slice() != &bytes[24..56] {
        return Err(invalid());
    }
    let entries: Vec<CacheEntry> = options().deserialize(&payload)?;
    if entries.is_empty() || entries.len() > 2 {
        return Err(invalid());
    }
    for e in &entries {
        crate::entry::validate_entry(e)?;
    }
    if entries.iter().any(|e| e.key != entries[0].key) {
        return Err(invalid());
    }
    Ok(Cell { entries })
}

/// Reads and decodes the cell stored at `path`. Equivalent to `read_cell_with_hook(path, || {})`;
/// see that function for the deletion-race guard (review R3).
pub fn read_cell(path: &Path) -> Option<Cell> {
    read_cell_with_hook(path, || {})
}

/// `read_cell`'s real implementation, with one added seam: `between_read_and_delete` runs after
/// a decode failure but before this function decides whether to unlink anything. Production code
/// only ever reaches this through `read_cell` (a no-op hook); the hook exists so
/// `crates/cache/tests/storage.rs` can deterministically reproduce the exact interleaving review
/// R3 describes (a valid replacement published in the window between this read and its cleanup)
/// without relying on real thread scheduling/timing. Not one of this task's five sanctioned
/// interfaces (`Cell`, `encode`, `decode`, `entry_path`, `read_cell`) -- `read_cell` itself is
/// unchanged for any caller that does not know this function exists.
///
/// Any failure -- I/O, an oversized file, a bad header, a checksum mismatch, or a
/// `decode`/`validate_entry` rejection -- is a cache miss (`None`). The file is deleted only if a
/// *fresh*, path-based metadata read taken immediately before removal still agrees, in both
/// length and last-write time, with what was captured from the *opened handle's* own metadata
/// before this function read anything (review R3; see the module doc for why this is a
/// best-effort proxy, not an exact identity check, and for the residual race it does not close).
/// A mismatch -- like a deletion failure, e.g. `path` naming a directory -- is silently treated
/// as "leave it alone": the lookup is still a miss, but nothing is removed.
pub fn read_cell_with_hook(path: &Path, between_read_and_delete: impl FnOnce()) -> Option<Cell> {
    let opened = std::fs::File::open(path).ok()?;
    let identity = {
        let m = opened.metadata().ok()?;
        (m.len(), m.last_write_time())
    };
    let result: Result<Cell, CacheError> = (|| {
        if identity.0 > HEADER_LEN + COMPRESSED_MAX {
            return Err(CacheError::Invalid("cache cell file exceeds the maximum on-disk size"));
        }
        let mut bytes = Vec::new();
        opened.take(HEADER_LEN + COMPRESSED_MAX + 1).read_to_end(&mut bytes)?;
        decode(&bytes)
    })();
    match result {
        Ok(cell) => Some(cell),
        Err(_) => {
            between_read_and_delete();
            if let Ok(fresh) = std::fs::metadata(path) {
                if (fresh.len(), fresh.last_write_time()) == identity {
                    let _ = std::fs::remove_file(path);
                }
            }
            None
        }
    }
}
