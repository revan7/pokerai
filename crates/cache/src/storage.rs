//! Bounded, checksummed binary storage for the on-disk street-solution cache (spec section 10.4):
//! a `Cell` holds the one or two `CacheEntry` values kept per structural key, `encode`/`decode`
//! convert a `Cell` to and from its on-disk bytes, `entry_path` derives the sharded file path for
//! a key digest, `write_atomic` publishes bytes at such a path (temp file in the same directory,
//! fsync, rename), and `read_cell` is the read side that turns any decode failure into a cache
//! miss with best-effort deletion of the offending file (CLAUDE.md section 6: "Read/decode error:
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
//!   writer thread), not this read path's -- so no lock is introduced here. **That lock now
//!   exists**: see "The publication lock" below, which closes both of those windows; the
//!   `(len, last_write_time)` proxy described here is kept only as a second guard against a
//!   writer outside this process.
//!
//! R4 (the storage-level corruption-case matrix) and R5 (test temp-directory hygiene) are test
//! changes only; see `crates/cache/tests/storage.rs`. R6 is a report correction only.
//!
//! ## The publication lock (task 6; R3's structural fix, owed by the orchestrator's ruling)
//!
//! R3's residual window described above is now closed. `write_atomic` (this task's publication
//! primitive) and `read_cell`'s corrupt-file cleanup take the *same* lock, `PUBLICATION`, so a
//! publication and a cleanup can never interleave:
//!
//! - Before it opens anything, `read_cell_core` registers a `ReadTicket` for the path it is about
//!   to read (`ReadGuard::new`). The ticket is removed when the read finishes, by the guard's
//!   `Drop` on every exit path -- including a panic and the early `File::open` failure.
//! - `write_atomic` writes and fsyncs its temp file *outside* the lock, then takes the lock and,
//!   under it, performs the `rename` that publishes the cell and marks every in-flight ticket for
//!   that same path `superseded`.
//! - `read_cell_core`, on a decode failure, takes the lock and -- under it -- decides and performs
//!   the `remove_file`. A ticket marked `superseded` means a publication landed on this path at
//!   some point after this read opened the file, so the bytes that failed to decode are no longer
//!   what the path names, and nothing is unlinked.
//!
//! The two orderings are therefore the only two possible ones, and both are safe: a publication
//! that commits before the cleanup's locked section marks the ticket (so the replacement is left
//! alone), and one that commits after it finds the corrupt file already unlinked and publishes on
//! top of nothing. Neither depends on the `(len, last_write_time)` proxy any more -- which matters
//! on Windows specifically, where `last_write_time` is frequently identical across writes issued
//! within the same system-clock tick, so a same-sized republish of a cell could genuinely collide
//! with it. The proxy is *kept* as a second, narrower guard for a writer this process cannot see
//! at all (a second app instance publishing into the same cache root, which holds no lock of
//! ours): it is no longer the only thing standing between a fresh publication and `remove_file`.
//! `read_cell`'s signature and observable behavior are unchanged -- an undisturbed corrupt file is
//! still gone once the call returns (`read_cell_still_deletes_an_undisturbed_corrupt_file`,
//! `cache_corrupt_entry_deleted`).
//!
//! Tickets are matched per path, by the path as given -- so a publication only ever silences the
//! cleanup of the *same* cell, never an unrelated one, and unrelated cells (and unrelated tests
//! running in parallel) never interfere. Both sides derive their path from `entry_path` against
//! the one cache root the writer was opened with, so the two spellings agree; a caller that
//! reached the same file through a differently spelled path would fall back to the identity proxy
//! alone, which is exactly the pre-task-6 behavior rather than a new failure mode.
//!
//! Scope of the lock (plan 4's "No mutex spans file I/O"): it is held across exactly two single
//! syscalls, the publishing `rename` and the cleanup `remove_file`, and never across an open,
//! read, write, fsync, encode, decode or validation -- those are precisely the operations that
//! rule is about, and making the two syscalls mutually exclusive is what the orchestrator's
//! ruling on R3 requires. Deletions that are *not* a corrupt-read cleanup (`WriteCommand::Delete`,
//! quota eviction, `quota::scan_index`'s garbage collection, `quota::sweep_temporaries`) all run
//! on the single writer thread, which is also the only publisher, so they are already serialized
//! against publication by being the same thread and take no lock.
//!
//! ## Fix round 2 (review `task-5-review-round-2.md`, N1)
//!
//! Round 1's `read_cell_with_hook` was a `pub fn`, which itself widened the production API for a
//! test-only seam (N1, Minor). It is now `read_cell_core` (the shared, always-compiled, private
//! implementation both `read_cell` and the test seam call) plus a `#[cfg(test)]`, non-public
//! `read_cell_with_hook` that only exists during `cargo test` and is only reachable from this
//! module's own `#[cfg(test)] mod tests` below -- never from `crates/cache/tests/storage.rs`
//! (a separate crate, which can only see this crate's real `pub` items). The R3 interleaving
//! regression that needs the hook moved into that unit-test module with it, with every one of
//! its assertions unchanged; `crates/cache/tests/storage.rs` now uses only the five sanctioned
//! public interfaces.

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

// --- the publication lock (task 6; see the module doc) ------------------------------------------

/// The in-flight `read_cell` calls that may still want to unlink the file they are reading. The
/// mutex guarding them is the publication lock itself: `write_atomic` holds it across its
/// `rename`, and a corrupt-read cleanup holds it across its decision and `remove_file`.
static PUBLICATION: std::sync::Mutex<Vec<ReadTicket>> = std::sync::Mutex::new(Vec::new());

struct ReadTicket {
    id: u64,
    path: PathBuf,
    /// Set by `write_atomic` when it publishes over `path` while this read is still in flight.
    superseded: bool,
}

/// Takes the publication lock, treating poisoning as recoverable: the guarded value is a plain
/// list of in-flight reads with no invariant a panicking thread could leave half-updated, and
/// refusing every publication and every cleanup for the rest of the process's life because some
/// unrelated thread panicked would be strictly worse than carrying on.
fn publication() -> std::sync::MutexGuard<'static, Vec<ReadTicket>> {
    PUBLICATION.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One in-flight read's registration in `PUBLICATION`, removed on `Drop` (so every exit path of
/// `read_cell_core`, including a panic, deregisters) or by `delete_unless_superseded`.
struct ReadGuard {
    id: u64,
    registered: bool,
}

impl ReadGuard {
    fn new(path: &Path) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        publication().push(ReadTicket { id, path: path.to_path_buf(), superseded: false });
        ReadGuard { id, registered: true }
    }

    /// Unlinks `path` unless a publication landed on it since this guard was taken. Both the
    /// decision and the `remove_file` run under the publication lock, which `write_atomic` also
    /// holds across its `rename`, so a cleanup and a publication are mutually exclusive (module
    /// doc). `identity` is task 5's `(len, last_write_time)` proxy, still required to agree as a
    /// second guard against a writer outside this process; a missing ticket is treated as
    /// superseded, i.e. as a reason not to delete.
    fn delete_unless_superseded(mut self, path: &Path, identity: (u64, u64)) {
        let mut tickets = publication();
        let superseded = tickets.iter().find(|t| t.id == self.id).is_none_or(|t| t.superseded);
        tickets.retain(|t| t.id != self.id);
        self.registered = false;
        if superseded {
            return;
        }
        if let Ok(fresh) = std::fs::metadata(path) {
            if (fresh.len(), fresh.last_write_time()) == identity {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

impl Drop for ReadGuard {
    fn drop(&mut self) {
        if self.registered {
            publication().retain(|t| t.id != self.id);
        }
    }
}

/// Publishes `bytes` at `path` atomically: writes them to a temp file in the *same* directory
/// (same volume, so the rename below is a true atomic replacement rather than a copy), fsyncs
/// that file, and renames it over `path`. A crash at any point leaves either the previous cell or
/// the new one, never a partially written file; a failure at any point removes the temp file and
/// leaves `path` untouched. The `rename` runs under the publication lock, which also marks every
/// in-flight `read_cell` on this path superseded, so a concurrent corrupt-read cleanup can never
/// unlink what this call just published (module doc).
///
/// # Errors
/// `CacheError::Invalid` if `path` has no parent directory. Propagates any I/O failure from
/// creating the directory, creating/writing/fsyncing the temp file, or the rename -- including a
/// full disk, a read-only cache root, and a `path` that names a directory.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), CacheError> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().ok_or(CacheError::Invalid("cache write path has no parent directory"))?;
    std::fs::create_dir_all(parent)?;
    // Unique per process *and* per call, so two publications of the same cell (or a crashed
    // earlier run's leftovers, which `quota::sweep_temporaries` clears) never collide on one
    // temp name; `create_new` below would refuse a collision rather than overwrite it.
    let tmp = path.with_extension(format!("{}.{}.tmp", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    let result = (|| -> Result<(), CacheError> {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        // The publication itself. Nothing above this line runs under the lock.
        let mut tickets = publication();
        std::fs::rename(&tmp, path)?;
        for t in tickets.iter_mut().filter(|t| t.path == path) {
            t.superseded = true;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
    result
}

/// Reads and decodes the cell stored at `path`. Equivalent to `read_cell_core(path, || {})`; see
/// that function for the deletion-race guard (review R3) and the module doc for the publication
/// lock that closes it (task 6).
pub fn read_cell(path: &Path) -> Option<Cell> {
    read_cell_core(path, || {})
}

/// `read_cell`'s real implementation, with one added seam: `between_read_and_delete` runs after
/// a decode failure but before this function decides whether to unlink anything. `read_cell`
/// (production, always compiled) calls this with a no-op hook; the only other caller is the
/// `#[cfg(test)]`-only `read_cell_with_hook` wrapper just below, used solely by this module's own
/// unit test to deterministically reproduce the exact interleaving review R3 describes (a valid
/// replacement published in the window between a read and its cleanup) without relying on real
/// thread scheduling/timing. Deliberately private (fix round 2, review N1: a `pub` hook-taking
/// function would itself widen the production API for a test-only seam) -- neither this function
/// nor `read_cell_with_hook` is one of this task's five sanctioned interfaces (`Cell`, `encode`,
/// `decode`, `entry_path`, `read_cell`), and `read_cell` itself is unchanged for every caller.
///
/// Any failure -- I/O, an oversized file, a bad header, a checksum mismatch, or a
/// `decode`/`validate_entry` rejection -- is a cache miss (`None`). The file is deleted only if
/// no publication landed on `path` while this call was reading it (the publication lock and the
/// `ReadGuard` registered below, task 6 -- see the module doc) *and* a fresh, path-based metadata
/// read taken under that lock still agrees, in both length and last-write time, with what was
/// captured from the *opened handle's* own metadata before this function read anything (review
/// R3's cross-process proxy). Either check failing -- like a deletion failure, e.g. `path` naming
/// a directory -- is silently treated as "leave it alone": the lookup is still a miss, but
/// nothing is removed.
fn read_cell_core(path: &Path, between_read_and_delete: impl FnOnce()) -> Option<Cell> {
    // Registered *before* the file is opened, so any publication on `path` from this point on is
    // visible to the cleanup decision below (task 6). Dropped on every exit path from here.
    let guard = ReadGuard::new(path);
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
            guard.delete_unless_superseded(path, identity);
            None
        }
    }
}

/// Test-only seam (fix round 2, review N1): exposes `read_cell_core`'s hook parameter, but only
/// under `#[cfg(test)]` and never `pub` -- it does not exist at all in a normal build, and is not
/// reachable from `crates/cache/tests/storage.rs` (a separate crate, which can only see this
/// crate's actual `pub` items) even during `cargo test`. Used only by this module's own
/// `#[cfg(test)] mod tests` below.
#[cfg(test)]
fn read_cell_with_hook(path: &Path, between_read_and_delete: impl FnOnce()) -> Option<Cell> {
    read_cell_core(path, between_read_and_delete)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ported from `crates/cache/tests/support/mod.rs::entry()` (fix round 2, review N1): that
    /// helper is written for the integration-test crate, where `cache::` refers to this crate as
    /// an external dependency; here, inside `cache` itself, the equivalent paths are `crate::`.
    /// A full, valid `CacheEntry` is what `read_cell_does_not_delete_a_replacement_published_between_read_and_cleanup`
    /// needs to prove its "replacement remains readable" assertion -- there is no smaller fixture
    /// that would still exercise a real `encode`/`read_cell` round trip.
    fn fixture_entry() -> crate::entry::CacheEntry {
        use crate::entry::{CacheEntry, SourceInputs};
        use crate::key::{KeyFields, Model, RakeKey, Rational};
        use proto::{Action, EffectiveTree, MaterializedNode, MenuSize, PlayerMenus, SideMenu, Street};

        let original = vec![proto::Card(46), proto::Card(21), proto::Card(0)];
        let r = core_ranges::parse_range("AA").unwrap();
        let (_, perm) = core_iso::canonicalize(&original, &[&r, &r]);
        let mut board = original.iter().map(|c| core_iso::apply(&perm, *c)).collect::<Vec<_>>();
        board.sort_by_key(|c| c.0);
        let r = core_iso::apply_range(&perm, &r);
        let hash = core_ranges::hash_scaled(&r);

        let mut materialized = Vec::new();
        for n in 0..6 {
            let street = [Street::Flop, Street::Turn, Street::River][n / 2];
            let actor = if n % 2 == 0 { "oop" } else { "ip" };
            let other = if n % 2 == 0 { "ip" } else { "oop" };
            materialized.push(MaterializedNode {
                path: vec![0; n],
                street,
                actor: actor.into(),
                actions: vec![Action::Check, Action::AllIn { to: 500 }],
                terminal_pots: vec![if n == 5 { Some(100) } else { None }, None],
            });
            let mut facing = vec![0; n];
            facing.push(1);
            materialized.push(MaterializedNode {
                path: facing,
                street,
                actor: other.into(),
                actions: vec![Action::Fold, Action::Call],
                terminal_pots: vec![Some(100), Some(1100)],
            });
        }
        materialized.sort_by(|a, b| a.path.cmp(&b.path));

        let side = SideMenu { bet: vec![MenuSize::AllIn], raise: vec![MenuSize::AllIn] };
        let menus = [Street::Flop, Street::Turn, Street::River]
            .into_iter()
            .map(|street| {
                (
                    street,
                    PlayerMenus { oop: side.clone(), ip: side.clone(), donk: if street == Street::Flop { None } else { Some(vec![]) } },
                )
            })
            .collect();
        let tree = EffectiveTree {
            rules_version: 3,
            template_id: "check_jam_test_v1".into(),
            root_street: Street::Flop,
            menus,
            add_allin_threshold: 0.0,
            force_allin_threshold: 0.0,
            merging_threshold: 0.0,
            wager_cap: 1,
            inserted: vec![],
            materialized,
        };

        let nodes = tree
            .materialized
            .iter()
            .filter(|n| n.street == Street::Flop)
            .enumerate()
            .map(|(k, n)| {
                let mut probs = vec![vec![0.0; 2]; 1326];
                let mut ev_chips = vec![vec![0.0; 2]; 1326];
                let mut available = vec![false; 1326];
                for i in 0..1326 {
                    available[i] = r.0[i] > 0.0;
                    probs[i] = if available[i] { vec![0.5, 0.5] } else { vec![0.0, 0.0] };
                    ev_chips[i] = if available[i] { vec![0.0, 10.0] } else { vec![0.0, 0.0] };
                }
                if matches!(n.actions[0], Action::Fold) {
                    for row in &mut ev_chips {
                        row[0] = 0.0;
                    }
                }
                for (i, row) in ev_chips.iter_mut().enumerate() {
                    if available[i] {
                        row[1] += k as f32;
                    }
                }
                proto::worker::NodeStrategy {
                    path: crate::entry::chip_path(&tree.materialized, &n.path).unwrap(),
                    actor: n.actor.clone(),
                    actions: n.actions.clone(),
                    probs,
                    ev_chips,
                    available,
                }
            })
            .collect::<Vec<_>>();

        let solution = proto::worker::StreetSolution {
            covered_paths: nodes.iter().map(|n| n.path.clone()).collect(),
            nodes,
            requested: 0,
            exploitability_chips: 0.4,
            iterations: 100,
            memory_bytes: 1024,
            mode: "f32".into(),
            locks_applied: 0,
            export: "street".into(),
        };
        let nodes = crate::entry::normalize(&solution, &tree, 100).unwrap();

        let fractions = tree
            .materialized
            .iter()
            .map(|n| {
                n.actions
                    .iter()
                    .map(|a| match a {
                        Action::Bet { to } | Action::Raise { to } | Action::AllIn { to } => Some(Rational::new(*to as u64, 100).unwrap()),
                        _ => None,
                    })
                    .collect()
            })
            .collect();

        CacheEntry {
            key: KeyFields {
                schema_version: 3,
                solver_commit: proto::worker::SOLVER_COMMIT.into(),
                adapter_version: 1,
                rules_version: 3,
                canonical_board: board,
                root_street: Street::Flop,
                spr_bucket: 81,
                tree_signature: "check_jam_test_v1".into(),
                rake: RakeKey::new(0.05, Rational::new(5000, 100_000).unwrap(), 1).unwrap(),
                range_hash_oop: hash,
                range_hash_ip: hash,
                model: Model::Baseline,
            },
            source: SourceInputs {
                pot: 100,
                stack_oop: 500,
                stack_ip: 500,
                spr: Rational::new(5, 1).unwrap(),
                bb_chips: 2,
                quantum_over_p: Rational::new(1, 100).unwrap(),
                cap_mchips: 5000,
                ranges: [r.clone(), r],
            },
            tree,
            fractions,
            covered_paths: nodes.iter().map(|n| n.path.clone()).collect(),
            nodes,
            exploitability_over_P: 0.004,
            target_bp: 50,
            iterations: 100,
            elapsed_ms: 10,
            memory_bytes: 1024,
            mode: "f32".into(),
            locks_applied: 0,
            export: "street".into(),
            reasons: vec![],
            created: 1,
            last_hit: 1,
        }
    }

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static UNIQUE: AtomicU64 = AtomicU64::new(0);
        let id = UNIQUE.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pokerai-cache-src-{label}-{}-{id}", std::process::id()));
        std::fs::create_dir(&dir).unwrap_or_else(|e| panic!("unique_temp_dir must get a fresh, exclusively-created directory at {dir:?}: {e}"));
        dir
    }

    /// The controlled-interleaving regression review R3 asked for: a valid replacement is
    /// published at `path` in the exact window between `read_cell`'s read (of the original
    /// corrupt bytes) and its cleanup decision, using the test-only `read_cell_with_hook` seam
    /// (this module's doc explains why a real thread race would be nondeterministic here). The
    /// replacement must survive -- not be deleted -- because its `(len, last_write_time)` no
    /// longer agrees with what was captured from the *original* file's opened handle.
    #[test]
    fn read_cell_does_not_delete_a_replacement_published_between_read_and_cleanup() {
        let dir = unique_temp_dir("race");
        let path = dir.join("cell.bin");
        std::fs::write(&path, b"not a cache header").unwrap();

        let e = fixture_entry();
        let replacement = encode(&Cell { entries: vec![e.clone()] }).unwrap();

        let got = read_cell_with_hook(&path, || {
            // Simulates a cache-writer's atomic publish landing in the window between this
            // read's decode failure and its cleanup step (review R3). A real writer would use a
            // temp-file-plus-rename publish (Task 6's job); an in-place overwrite is enough here
            // to change this path's (len, last_write_time) identity, which is all the guard
            // checks.
            std::fs::write(&path, &replacement).unwrap();
        });
        assert!(got.is_none(), "the original corrupt bytes must still be reported as a miss");
        assert!(path.exists(), "the concurrently published replacement must survive cleanup");
        assert_eq!(std::fs::read(&path).unwrap(), replacement, "cleanup must not have touched the replacement's bytes");

        let reread = read_cell(&path).expect("the surviving replacement must still be a hit");
        assert_eq!(reread.entries[0].key, e.key);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The structural regression task 6 owes review R3 (orchestrator ruling): a *real*
    /// publication through `write_atomic`, landing in the exact window between a corrupt read and
    /// its cleanup, is never unlinked -- even when the published file's `(len, last_write_time)`
    /// is made to agree *exactly* with the corrupt file's, i.e. even in the one case task 5's
    /// best-effort proxy cannot distinguish (and which is not exotic on Windows, where two writes
    /// in the same system-clock tick share a last-write time). The interleaving is driven by the
    /// crate-internal hook, never by timing, and the assertion inside the hook proves the proxy
    /// really does agree -- so the only thing that can be keeping the replacement alive is the
    /// publication lock and the ticket `write_atomic` marked under it.
    #[test]
    fn a_publication_racing_a_corrupt_read_survives_even_when_len_and_mtime_agree() {
        let dir = unique_temp_dir("publish-race");
        let path = dir.join("cell.bin");
        let e = fixture_entry();
        let replacement = encode(&Cell { entries: vec![e.clone()] }).unwrap();
        // Corrupt bytes of exactly the replacement's length: zeros fail the magic check.
        std::fs::write(&path, vec![0_u8; replacement.len()]).unwrap();
        let before = std::fs::metadata(&path).unwrap();
        let (len, modified) = (before.len(), before.modified().unwrap());

        let got = read_cell_with_hook(&path, || {
            write_atomic(&path, &replacement).unwrap();
            let published = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            published.set_times(std::fs::FileTimes::new().set_modified(modified)).unwrap();
            drop(published);
            let fresh = std::fs::metadata(&path).unwrap();
            assert_eq!(
                (fresh.len(), fresh.modified().unwrap()),
                (len, modified),
                "the published replacement's identity must match the corrupt file's, or this test is not exercising the lock"
            );
        });

        assert!(got.is_none(), "the corrupt bytes this call actually read are still a miss");
        assert_eq!(std::fs::read(&path).unwrap(), replacement, "the published replacement must survive the cleanup");
        let reread = read_cell(&path).expect("the surviving replacement must still be a hit");
        assert_eq!(reread.entries[0].key, e.key);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The other ordering: a cleanup that commits first unlinks only the corrupt bytes it read,
    /// and a publication arriving afterwards still lands. Together with the test above this
    /// covers both of the two orderings the publication lock allows.
    #[test]
    fn a_publication_after_a_cleanup_still_lands() {
        let dir = unique_temp_dir("publish-after");
        let path = dir.join("cell.bin");
        std::fs::write(&path, b"not a cache header").unwrap();
        assert!(read_cell(&path).is_none());
        assert!(!path.exists(), "the corrupt file it read is unlinked");

        let e = fixture_entry();
        let bytes = encode(&Cell { entries: vec![e.clone()] }).unwrap();
        write_atomic(&path, &bytes).unwrap();
        assert_eq!(read_cell(&path).expect("the later publication must be a hit").entries[0].key, e.key);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
