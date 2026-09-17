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
//! The payload is a `Vec<DiskEntry>` of one or two entries, each split into its `metadata`
//! (everything in `CacheEntry` except `nodes`, JSON-encoded so `validate_entry`'s own JSON-wire
//! numeric widening/domain checks -- `crate::entry`'s `narrow_checked`/`widen_checked` -- run on
//! decode exactly as they do for any other JSON boundary) and `nodes` (the per-node probability/
//! EV matrices, bincode-encoded so they keep bincode's native, half-the-size `f32` wire width
//! rather than JSON's widened `f64`, per `crate::entry`'s own `is_human_readable()` split).
//! `encode` re-runs `crate::entry::validate_entry` on every entry before it is ever written, and
//! `decode` re-runs it on every entry it reads back -- storage never persists or discloses an
//! entry that fails the structural/numeric invariants `validate_entry` already enforces
//! elsewhere, and a corrupt payload that happens to keep a valid checksum is still caught by that
//! content check, not only by the sha256 (Interfaces: "Boundaries are per cell, including its two
//! entries").
//!
//! Cells whose entries do not share one structural key digest are rejected on both `encode` (a
//! writer invariant, review m4: two entries that disagree on their own key have no business being
//! written into the same cell) and `decode` (the same check re-run against whatever was actually
//! read back).

use crate::entry::CachedNode;
use crate::CacheError;
use bincode::Options;
use sha2::{Digest, Sha256};
use std::io::Read;
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
    pub entries: Vec<crate::entry::CacheEntry>,
}

/// The on-disk shape of one cell entry: `metadata` is `CacheEntry` with `nodes` cleared,
/// JSON-encoded; `nodes` is the entry's own node list, kept in its native (bincode) matrix
/// width. Never exported: callers only ever see a whole `Cell` via `encode`/`decode`.
#[derive(serde::Serialize, serde::Deserialize)]
struct DiskEntry {
    metadata: Vec<u8>,
    nodes: Vec<CachedNode>,
}

/// A `std::io::Write` sink that refuses to grow past `limit` bytes, so a serializer writing into
/// it (rather than allocating its own unbounded buffer first) is itself bounded -- used to cap
/// the metadata JSON encode below at `DECODED_MAX` without first allocating an oversized buffer
/// for a malformed or hostile `CacheEntry`.
struct CappedBuffer {
    bytes: Vec<u8>,
    limit: usize,
}

impl std::io::Write for CappedBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "cache size limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// JSON-encodes `value` into a buffer capped at `DECODED_MAX` bytes, failing rather than
/// allocating without bound.
fn bounded_json<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, CacheError> {
    let mut out = CappedBuffer { bytes: Vec::new(), limit: DECODED_MAX as usize };
    serde_json::to_writer(&mut out, value)?;
    Ok(out.bytes)
}

/// The bincode configuration the whole cell payload (the `Vec<DiskEntry>`) is serialized and
/// deserialized under: fixed-width (not varint) integers, little-endian, decode capped at
/// `DECODED_MAX`, and no trailing bytes tolerated after the last value. `encode` and `decode`
/// both call this rather than each building their own configuration, so the two directions can
/// never silently drift apart.
fn options() -> impl Options {
    bincode::DefaultOptions::new().with_fixint_encoding().with_little_endian().with_limit(DECODED_MAX).reject_trailing_bytes()
}

/// The sharded on-disk path for a cell keyed by `key` (a `crate::key::KeyFields::digest()`),
/// rooted at `dir` (spec section 10.4: `<cache root>\v3\<key[0..2]>\<key>.bin`, `dir` already
/// being that `v3` root).
pub fn entry_path(dir: &Path, key: [u8; 32]) -> PathBuf {
    let hex = key.iter().map(|b| format!("{b:02x}")).collect::<String>();
    dir.join(&hex[..2]).join(format!("{hex}.bin"))
}

/// Encodes `cell` to its on-disk bytes: validates every entry (`crate::entry::validate_entry`),
/// requires every entry to share the same structural key digest, JSON-encodes each entry's
/// metadata (bounded) and keeps its nodes in native bincode width, checks the decoded payload
/// against `DECODED_MAX` before serializing it, zstd-compresses it, checks the compressed result
/// against `COMPRESSED_MAX`, and prefixes the fixed 56-byte header (magic, schema/proto version,
/// compressed/decoded length, decoded-payload sha256).
///
/// # Errors
/// `CacheError::Invalid` if `cell` is empty or has more than two entries, if its entries disagree
/// on their key digest, if any entry fails `validate_entry`, or if the decoded or compressed
/// payload exceeds its bound. Propagates any JSON, bincode or I/O (zstd) failure.
pub fn encode(cell: &Cell) -> Result<Vec<u8>, CacheError> {
    if cell.entries.is_empty() || cell.entries.len() > 2 {
        return Err(CacheError::Invalid("cell entry count must be 1 or 2"));
    }
    // review m4: the digest agreement is a writer invariant too, not only a reader check.
    let digest = cell.entries[0].key.digest();
    if cell.entries.iter().any(|e| e.key.digest() != digest) {
        return Err(CacheError::Invalid("cell entries disagree on key digest"));
    }
    let mut disk = Vec::with_capacity(cell.entries.len());
    for e in &cell.entries {
        crate::entry::validate_entry(e)?;
        let mut metadata = e.clone();
        metadata.nodes.clear();
        disk.push(DiskEntry { metadata: bounded_json(&metadata)?, nodes: e.nodes.clone() });
    }
    if options().serialized_size(&disk)? > DECODED_MAX {
        return Err(CacheError::Invalid("decoded cell payload exceeds DECODED_MAX"));
    }
    let payload = options().serialize(&disk)?;
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
/// re-validates every entry (`crate::entry::validate_entry`) and requires them to share one key
/// digest.
///
/// # Errors
/// `CacheError::Invalid` (wrapped by the local `invalid` closure) for any header, length,
/// checksum, entry-count or key-digest mismatch. Propagates any I/O (zstd) or bincode failure
/// from a malformed compressed stream or payload.
pub fn decode(bytes: &[u8]) -> Result<Cell, CacheError> {
    let invalid = || CacheError::Invalid("cache cell header or payload is invalid");
    let want_versions: Vec<u8> = [SCHEMA_VERSION.to_le_bytes(), PROTO_VERSION.to_le_bytes()].concat();
    if bytes.len() < HEADER_LEN as usize || &bytes[..4] != MAGIC || &bytes[4..8] != want_versions.as_slice() {
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
    let disk: Vec<DiskEntry> = options().deserialize(&payload)?;
    if disk.is_empty() || disk.len() > 2 {
        return Err(invalid());
    }
    let mut entries = Vec::with_capacity(disk.len());
    for d in disk {
        let mut e: crate::entry::CacheEntry = serde_json::from_slice(&d.metadata)?;
        e.nodes = d.nodes;
        crate::entry::validate_entry(&e)?;
        entries.push(e);
    }
    let digest = entries[0].key.digest();
    if entries.iter().any(|e| e.key.digest() != digest) {
        return Err(invalid());
    }
    Ok(Cell { entries })
}

/// Reads and decodes the cell stored at `path`. Any failure -- I/O, an oversized file, a bad
/// header, a checksum mismatch, or a `decode`/`validate_entry` rejection -- is a cache miss
/// (`None`), and the offending file is deleted on a best-effort basis (CLAUDE.md section 6:
/// "Read/decode error: miss, best-effort delete"; a deletion failure, e.g. `path` naming a
/// directory rather than a file, is itself ignored -- the lookup is still a miss).
pub fn read_cell(path: &Path) -> Option<Cell> {
    let result = (|| -> Result<Cell, CacheError> {
        let file = std::fs::File::open(path)?;
        if file.metadata()?.len() > HEADER_LEN + COMPRESSED_MAX {
            return Err(CacheError::Invalid("cache cell file exceeds the maximum on-disk size"));
        }
        let mut bytes = Vec::new();
        file.take(HEADER_LEN + COMPRESSED_MAX + 1).read_to_end(&mut bytes)?;
        decode(&bytes)
    })();
    match result {
        Ok(cell) => Some(cell),
        Err(_) => {
            let _ = std::fs::remove_file(path);
            None
        }
    }
}
