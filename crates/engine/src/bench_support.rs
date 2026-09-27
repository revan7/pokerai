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

/// Recomputes each bundle's sha256 (and byte length) from the chart file the store actually
/// loaded (`charts_dir/<name>.json`), so a chart that was re-transcribed, hand-edited or
/// replaced after the freeze aborts generation instead of silently benchmarking stale data. A
/// bundle file that cannot even be read (missing, permission-denied) is reported by name rather
/// than panicking.
pub fn verify_source_lock(lock: &SourceLock, charts_dir: &std::path::Path) -> Result<(), String> {
    use sha2::Digest;
    for bundle in &lock.bundles {
        let path = charts_dir.join(format!("{}.json", bundle.name));
        let raw = std::fs::read(&path).map_err(|e| format!("{}: {e}", bundle.name))?;
        let actual = hex::encode(sha2::Sha256::digest(&raw));
        if actual != bundle.sha256 || raw.len() as u64 != bundle.bytes {
            return Err(format!("{} drifted from the frozen source lock", bundle.name));
        }
    }
    Ok(())
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
        let lock = load_source_lock(&repo_root().join("bench/spots/sources.json")).unwrap();
        let err = verify_source_lock(&lock, Path::new("no/such/charts/dir")).unwrap_err();
        assert!(err.contains("pokercoaching_100"), "{err}");
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
