//! R1: the CLI's option parser must distinguish a genuinely omitted optional flag (default
//! applies) from a flag present without a value, an unknown argument, or a malformed number —
//! each of the latter three is rejected with a message naming the offending item and a nonzero
//! exit, and nothing is ever written or printed as JSON for a rejected request.
//!
//! These are black-box tests against the compiled `bench` binary (`CARGO_BIN_EXE_bench`, set by
//! cargo for integration tests with no extra dependency) because the defect and its fix are about
//! observable process behavior: exit status, stdout/stderr content, and filesystem side effects.

use std::process::{Command, Output};

fn bench(args: &[&str], cwd: &std::path::Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bench")).args(args).current_dir(cwd).output().expect("spawn bench")
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("bench_cli_{tag}_{}_{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// R1: `bench materialize ... --prefix` with `--prefix` the last token (no value following) must be
/// rejected, not silently treated as an omitted `--prefix` (which would default to the empty prefix).
#[test]
fn trailing_prefix_flag_is_rejected() {
    let dir = temp_dir("trailing_prefix");
    let out = bench(&["materialize", "--template", "river_oracle_v1", "--pot", "100", "--eff", "100", "--prefix"], &dir);
    assert!(!out.status.success(), "expected a nonzero exit, got {:?}", out.status);
    assert!(out.stdout.is_empty(), "no JSON must be printed on rejection, got {:?}", String::from_utf8_lossy(&out.stdout));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--prefix"), "error must name the offending flag: {stderr}");
    std::fs::remove_dir_all(&dir).ok();
}

/// R1: `bench gen-spots --source` with `--source` the last token must be rejected, not silently
/// default to `r8`. No fixture files may be written on rejection.
#[test]
fn trailing_source_flag_is_rejected() {
    let dir = temp_dir("trailing_source");
    let out = bench(&["gen-spots", "--source"], &dir);
    assert!(!out.status.success(), "expected a nonzero exit, got {:?}", out.status);
    assert!(out.stdout.is_empty(), "no files must be reported written on rejection, got {:?}", String::from_utf8_lossy(&out.stdout));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--source"), "error must name the offending flag: {stderr}");
    assert!(!dir.join("bench").exists(), "no output directory must be created on rejection");
    std::fs::remove_dir_all(&dir).ok();
}

/// R1: `bench gen-spots --source r8 --out` with `--out` the last token must be rejected, not
/// silently default to `bench/spots`. No fixture files may be written on rejection.
#[test]
fn trailing_out_flag_is_rejected() {
    let dir = temp_dir("trailing_out");
    let out = bench(&["gen-spots", "--source", "r8", "--out"], &dir);
    assert!(!out.status.success(), "expected a nonzero exit, got {:?}", out.status);
    assert!(out.stdout.is_empty(), "no files must be reported written on rejection, got {:?}", String::from_utf8_lossy(&out.stdout));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--out"), "error must name the offending flag: {stderr}");
    assert!(!dir.join("bench").exists(), "no output directory must be created on rejection");
    std::fs::remove_dir_all(&dir).ok();
}

/// R1: an unrecognized option must be rejected by name, not silently ignored.
#[test]
fn unknown_option_is_rejected() {
    let dir = temp_dir("unknown_option");
    let out = bench(&["materialize", "--template", "river_oracle_v1", "--pot", "100", "--eff", "100", "--bogus", "5"], &dir);
    assert!(!out.status.success(), "expected a nonzero exit, got {:?}", out.status);
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--bogus"), "error must name the offending option: {stderr}");
    std::fs::remove_dir_all(&dir).ok();
}

/// R1: a malformed numeric option must be rejected by name and value, not silently coerced to 0.
#[test]
fn malformed_number_is_rejected() {
    let dir = temp_dir("malformed_number");
    let out = bench(&["materialize", "--template", "river_oracle_v1", "--pot", "abc", "--eff", "100"], &dir);
    assert!(!out.status.success(), "expected a nonzero exit, got {:?}", out.status);
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--pot") && stderr.contains("abc"), "error must name the offending option and value: {stderr}");
    std::fs::remove_dir_all(&dir).ok();
}

/// R1 control: a genuinely omitted `--prefix` still applies the empty-prefix default and succeeds.
#[test]
fn materialize_without_prefix_uses_the_empty_prefix_default() {
    let dir = temp_dir("materialize_control");
    let out = bench(&["materialize", "--template", "river_oracle_v1", "--pot", "100", "--eff", "100"], &dir);
    assert!(out.status.success(), "expected success, got {:?}, stderr={}", out.status, String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON on stdout");
    assert_eq!(v["decision_path"], serde_json::json!([]));
    std::fs::remove_dir_all(&dir).ok();
}

/// R1 control: genuinely omitted `--source`/`--out` still apply their defaults and succeed.
#[test]
fn gen_spots_without_optional_flags_uses_defaults() {
    let dir = temp_dir("gen_spots_control");
    let out = bench(&["gen-spots"], &dir);
    assert!(out.status.success(), "expected success, got {:?}, stderr={}", out.status, String::from_utf8_lossy(&out.stderr));
    assert!(dir.join("bench").join("spots").join("river_std.json").exists(), "default --out=bench/spots must be used");
    std::fs::remove_dir_all(&dir).ok();
}
