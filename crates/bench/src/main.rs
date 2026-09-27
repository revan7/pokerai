use bench::{gen_spots, materialize, report, runner, suite};
use engine::worker::link::WorkerLink;
use std::collections::HashMap;

/// Parses `--flag value` pairs from the arguments following the subcommand. Every flag must be
/// recognized (`known`), have an explicit value that is not itself another `--flag` or the end of
/// input, and appear at most once.
///
/// R1: this is the CLI's one input boundary. The previous helper looked up `position(--flag)` and
/// took `args.get(i + 1)`, so a trailing flag (present, no value) produced exactly the same `None`
/// as a flag never given at all, and an unrecognized flag or an unparseable number was silently
/// discarded to a default. A request that fails here returns an error naming the offending token
/// before anything is materialized, generated or written.
fn parse_options(args: &[String], known: &[&str]) -> Result<HashMap<String, String>, String> {
    let mut map = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        let tok = &args[i];
        let name = tok.strip_prefix("--").ok_or_else(|| format!("unexpected argument {tok:?}: expected a --flag"))?;
        if !known.contains(&name) { return Err(format!("unknown option --{name}")); }
        let value = match args.get(i + 1) {
            Some(v) if !v.starts_with("--") => v.clone(),
            _ => return Err(format!("--{name} requires a value but none was given")),
        };
        if map.insert(name.to_string(), value).is_some() { return Err(format!("--{name} was given more than once")); }
        i += 2;
    }
    Ok(map)
}

/// A required option: missing entirely (never given) is an error naming the flag.
fn required(map: &HashMap<String, String>, name: &str) -> Result<String, String> {
    map.get(name).cloned().ok_or_else(|| format!("missing required option --{name}"))
}

/// An optional option: applies `default` only when the flag was never given at all
/// (`parse_options` already rejected a flag present without a value).
fn optional(map: &HashMap<String, String>, name: &str, default: &str) -> String {
    map.get(name).cloned().unwrap_or_else(|| default.to_string())
}

/// A required numeric option: a value that fails to parse is rejected by name and by the offending
/// text, never silently coerced to 0.
fn required_u32(map: &HashMap<String, String>, name: &str) -> Result<u32, String> {
    let raw = required(map, name)?;
    raw.parse::<u32>().map_err(|_| format!("--{name} must be a non-negative integer, got {raw:?}"))
}

/// An optional numeric option: applies `default` only when the flag was never given at all; a value
/// that is present but fails to parse is rejected by name and by the offending text, never silently
/// replaced by the default (the same rule `required_u32` applies to a required flag).
fn optional_u8(map: &HashMap<String, String>, name: &str, default: u8) -> Result<u8, String> {
    match map.get(name) {
        None => Ok(default),
        Some(raw) => raw.parse::<u8>().map_err(|_| format!("--{name} must be an integer 0-255, got {raw:?}")),
    }
}
fn optional_u32(map: &HashMap<String, String>, name: &str, default: u32) -> Result<u32, String> {
    match map.get(name) {
        None => Ok(default),
        Some(raw) => raw.parse::<u32>().map_err(|_| format!("--{name} must be a non-negative integer, got {raw:?}")),
    }
}

fn cmd_materialize(rest: &[String]) -> Result<String, String> {
    let map = parse_options(rest, &["template", "pot", "eff", "prefix"])?;
    let template = required(&map, "template")?;
    let pot = required_u32(&map, "pot")?;
    let eff = required_u32(&map, "eff")?;
    let prefix = optional(&map, "prefix", "");
    materialize::run(&template, pot, eff, &prefix)
}

/// Returns the process exit code for the (already option-validated) `gen-spots` run: 0 if every
/// suite generated and saved, 2 if any suite failed (matching a per-command execution error).
fn cmd_gen_spots(rest: &[String]) -> Result<i32, String> {
    let map = parse_options(rest, &["source", "out"])?;
    let source = optional(&map, "source", "r8");
    let out = std::path::PathBuf::from(optional(&map, "out", "bench/spots"));
    let mut code = 0;
    for suite in ["river_std", "river_min", "turn_std", "turn_min"] {
        match gen_spots::generate(suite, &source).and_then(|s| s.save(&out.join(format!("{suite}.json")))) {
            Ok(()) => println!("wrote {suite}"),
            Err(e) => { eprintln!("{e}"); code = 2; }
        }
    }
    Ok(code)
}

/// Runs `--suite` (default `river_std`) `--reps` times (default 5) per spot against a fresh
/// release-worker process per spot (cold = the first rep in that fresh process), measures a §13.5
/// cancel latency against the suite's first spot, and appends the resulting report section to
/// `--out/<BENCH_DATE or 2026-09-10>-i7-13700K.md` (default `--out docs/bench`).
///
/// The worker binary is never built here: it is discovered through `POKERAI_WORKER` or the
/// workspace's default release path, and a worker that fails to spawn (most commonly because it was
/// never built) is reported by name with a pointer to `cargo build --release -p solver-worker`
/// rather than silently producing an empty or partial report.
///
/// Every appended section carries the run's metadata (`report::RunMetadata`): the start time, the CPU,
/// the toolchain, the worker binary and its `ready` identity (identical across the per-spot spawns, or
/// the run fails), the target and the budgets. A rejected solve, a missing reply or any other protocol
/// failure (`runner::RunError`) aborts the run with exit code 2 and writes nothing: a partial report
/// would present an incomplete suite as a measurement.
fn cmd_run(rest: &[String]) -> Result<(), String> {
    let map = parse_options(rest, &["suite", "threads", "reps", "out"])?;
    let suite_name = optional(&map, "suite", "river_std");
    let threads = optional_u8(&map, "threads", 16)?;
    let reps = optional_u32(&map, "reps", 5)?;
    let out = std::path::PathBuf::from(optional(&map, "out", "docs/bench"));
    let date = std::env::var("BENCH_DATE").unwrap_or_else(|_| "2026-09-10".into());
    if !is_iso_date(&date) {
        return Err(format!("BENCH_DATE must be a YYYY-MM-DD date, got {date:?}"));
    }
    let suite_path = std::path::PathBuf::from("bench/spots").join(format!("{suite_name}.json"));
    let suite = suite::Suite::load(&suite_path)?;
    if suite.spots.is_empty() {
        return Err(format!("{}: the suite has no spots", suite_path.display()));
    }
    let exe = std::env::var("POKERAI_WORKER").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from("target/release/solver-worker.exe"));
    let started = unix_now().map_or_else(|e| format!("unavailable ({e})"), report::utc_timestamp);
    let clock = engine::clock::SystemClock::new();
    let mut rep = report::Report::new(&suite_name, threads, reps);
    let mut ready: Option<proto::worker::Ready> = None;
    for (index, spot) in suite.spots.iter().enumerate() {
        // cold = the first rep in a freshly spawned process; a fresh process is used per spot so a
        // later spot's cold rep is never warmed by an earlier spot's solve.
        let mut worker = engine::worker::process::ProcessWorker::spawn(&exe, threads).map_err(|e| {
            format!("{}: {e} -- build the release worker first (cargo build --release -p solver-worker) or point POKERAI_WORKER at an existing one", exe.display())
        })?;
        let this = worker.ready().cloned().ok_or_else(|| format!("{}: a spawned worker has no validated ready line", exe.display()))?;
        match &ready {
            None => ready = Some(this),
            Some(first) if *first != this => {
                return Err(format!("{}: the worker's ready identity changed between spawns ({}; then {})", exe.display(), report::ready_identity(first), report::ready_identity(&this)))
            }
            Some(_) => {}
        }
        for r in 1..=reps {
            let res = runner::run_spot(&mut worker, &clock, spot, r, r == 1).map_err(|e| format!("suite {suite_name}, spot {}, rep {r}: {e}", spot.id))?;
            println!("{} rep {} {} {} ms", res.spot, r, res.status, res.wall_ms);
            rep.push(res);
        }
        if index == 0 {
            let outcome = runner::cancel_latency(&mut worker, &clock, spot).map_err(|e| format!("suite {suite_name}, spot {}: cancel-latency probe: {e}", spot.id))?;
            println!("{} cancel probe {outcome:?}", spot.id);
            rep.set_cancel(outcome);
        }
        worker.kill();
    }
    let ready = ready.expect("the suite has at least one spot, so a worker was spawned");
    rep.set_metadata(run_metadata(started, &exe, &ready, &suite, &suite_path));
    let path = out.join(format!("{date}-i7-13700K.md"));
    rep.append_to(&path, &date).map_err(|e| format!("{}: {e}", path.display()))
}

fn unix_now() -> Result<u64, String> {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).map_err(|e| e.to_string())
}

/// The run's metadata, each field captured now or marked unavailable with the reason, never inferred.
fn run_metadata(started: String, exe: &std::path::Path, ready: &proto::worker::Ready, suite: &suite::Suite, suite_path: &std::path::Path) -> report::RunMetadata {
    let distinct = |values: Vec<String>| {
        let mut v = values;
        v.sort();
        v.dedup();
        v.join(", ")
    };
    let streets: Vec<proto::Street> = {
        let mut s: Vec<proto::Street> = suite.spots.iter().map(|s| s.root_street).collect();
        s.dedup();
        s
    };
    report::RunMetadata {
        recorded_at: started,
        hardware: hardware_identity(),
        toolchain: toolchain_identity(),
        worker_binary: worker_binary_identity(exe),
        worker_ready: report::ready_identity(ready),
        target: format!(
            "target_bp {} (reached iff raw exploitability_chips x 10000 <= target_bp x pot)",
            distinct(suite.spots.iter().map(|s| s.target_bp.to_string()).collect())
        ),
        budgets: streets.iter().map(|s| format!("{s:?}: {}", runner::budget_summary(*s))).collect::<Vec<_>>().join("; "),
        spots: format!(
            "{} spots from {} (templates {}; range source {})",
            suite.spots.len(),
            suite_path.display(),
            distinct(suite.spots.iter().map(|s| s.template_id.clone()).collect()),
            distinct(suite.spots.iter().map(|s| s.range_source.clone()).collect())
        ),
    }
}

/// Runs a read-only query tool and returns its stdout, or why it could not.
fn command_stdout(program: &str, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new(program).args(args).output().map_err(|e| format!("{program}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{program} exited with {}", out.status));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The CPU name (Windows registry, read with `reg query`) and the logical processor count.
fn hardware_identity() -> String {
    let cpu = command_stdout("reg", &[r"query", r"HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0", "/v", "ProcessorNameString"])
        .and_then(|listing| parse_processor_name(&listing).ok_or_else(|| "no ProcessorNameString value".to_string()))
        .unwrap_or_else(|e| format!("CPU name unavailable ({e})"));
    let logical = std::thread::available_parallelism().map_or_else(|e| format!("logical processors unavailable ({e})"), |n| format!("{n} logical processors"));
    format!("{cpu}; {logical}")
}

/// The toolchain `rustc -vV` resolves to in the working directory at run time (the pinned
/// `rust-toolchain.toml` of the workspace, unless overridden), and the `bench` binary's own compile-time
/// target and profile.
fn toolchain_identity() -> String {
    let rustc = command_stdout("rustc", &["-vV"])
        .and_then(|vv| parse_rustc_version(&vv).ok_or_else(|| "unrecognized rustc -vV output".to_string()))
        .unwrap_or_else(|e| format!("rustc unavailable ({e})"));
    let env = if cfg!(target_env = "msvc") { "msvc" } else if cfg!(target_env = "gnu") { "gnu" } else { "other" };
    let profile = if cfg!(debug_assertions) { "debug assertions on" } else { "release (no debug assertions)" };
    let avx2 = if cfg!(target_feature = "avx2") { "avx2 on" } else { "avx2 off" };
    format!(
        "{rustc} (resolved in the working directory at run time); bench binary {}-{}-{env}, {profile}, {avx2}",
        std::env::consts::ARCH,
        std::env::consts::OS
    )
}

/// The worker executable's path, size and modification time, plus the V1 toolchain selection record as it
/// reads at run time. The worker's `ready` line carries no compiler identity, so none is claimed for it.
fn worker_binary_identity(exe: &std::path::Path) -> String {
    let file = match std::fs::metadata(exe) {
        Ok(m) => {
            let modified = m
                .modified()
                .map_err(|e| e.to_string())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string()))
                .map_or_else(|e| format!("modified time unavailable ({e})"), |d| format!("modified {}", report::utc_timestamp(d.as_secs())));
            format!("{} bytes, {modified}", m.len())
        }
        Err(e) => format!("file metadata unavailable ({e})"),
    };
    let selection = std::fs::read_to_string("docs/bench/worker-toolchain.json")
        .map_err(|e| e.to_string())
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).map_err(|e| e.to_string()))
        .map(|v| format!("{} / {}", v["worker_toolchain"].as_str().unwrap_or("?"), v["worker_target"].as_str().unwrap_or("?")))
        .unwrap_or_else(|e| format!("unavailable ({e})"));
    format!(
        "{} ({file}); compiler identity not reported by the worker; V1 selection record docs/bench/worker-toolchain.json at run time: {selection}",
        exe.display()
    )
}

/// The CPU name from a `reg query ... /v ProcessorNameString` listing: the text after `REG_SZ` on the
/// value's line, `None` when there is no such line or the name is empty.
fn parse_processor_name(reg_output: &str) -> Option<String> {
    reg_output
        .lines()
        .find(|l| l.trim_start().starts_with("ProcessorNameString"))
        .and_then(|l| l.split_once("REG_SZ"))
        .map(|(_, name)| name.trim().to_string())
        .filter(|name| !name.is_empty())
}

/// `rustc -vV` summarized: its version line, host triple and LLVM version; `None` unless both the version
/// line and the `host:` line are present.
fn parse_rustc_version(output: &str) -> Option<String> {
    let version = output.lines().next().filter(|l| l.starts_with("rustc "))?.trim();
    let field = |key: &str| output.lines().find_map(|l| l.strip_prefix(key)).map(str::trim);
    let host = field("host:")?;
    Some(match field("LLVM version:") {
        Some(llvm) => format!("{version}, host {host}, LLVM {llvm}"),
        None => format!("{version}, host {host}"),
    })
}

/// `BENCH_DATE` names the report file and dates its frontmatter: a `YYYY-MM-DD` date with a month in
/// 1-12 and a day in 1-31.
fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| b[r.clone()].iter().all(u8::is_ascii_digit).then(|| s[r].parse::<u32>().ok()).flatten();
    matches!((digits(0..4), digits(5..7), digits(8..10)), (Some(_), Some(1..=12), Some(1..=31)))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (code, message) = match args.get(1).map(String::as_str) {
        Some("materialize") => match cmd_materialize(&args[2..]) { Ok(json) => { println!("{json}"); (0, None) } Err(e) => (2, Some(e)) },
        Some("gen-spots") => match cmd_gen_spots(&args[2..]) { Ok(code) => (code, None), Err(e) => (2, Some(e)) },
        Some("run") => match cmd_run(&args[2..]) { Ok(()) => (0, None), Err(e) => (2, Some(e)) },
        _ => (1, Some("usage: bench materialize --template ID --pot P --eff E [--prefix ...] | bench gen-spots [--source r8] [--out DIR] | bench run --suite river_std|river_min|turn_std|turn_min [--threads N] [--reps R] [--out DIR]".to_string())),
    };
    if let Some(m) = message { eprintln!("{m}"); }
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(strs: &[&str]) -> Vec<String> { strs.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn a_trailing_flag_with_no_value_is_rejected() {
        let e = parse_options(&s(&["--template", "x", "--prefix"]), &["template", "prefix"]).unwrap_err();
        assert!(e.contains("--prefix"), "{e}");
    }

    #[test]
    fn a_flag_immediately_followed_by_another_flag_has_no_value() {
        let e = parse_options(&s(&["--source", "--out", "dir"]), &["source", "out"]).unwrap_err();
        assert!(e.contains("--source"), "{e}");
    }

    #[test]
    fn an_unknown_flag_is_rejected_by_name() {
        let e = parse_options(&s(&["--bogus", "5"]), &["template"]).unwrap_err();
        assert!(e.contains("--bogus"), "{e}");
    }

    #[test]
    fn a_bare_non_flag_token_is_rejected() {
        let e = parse_options(&s(&["oops"]), &["template"]).unwrap_err();
        assert!(e.contains("oops"), "{e}");
    }

    #[test]
    fn a_genuinely_omitted_optional_flag_gets_the_default() {
        let map = parse_options(&s(&["--template", "x"]), &["template", "prefix"]).unwrap();
        assert_eq!(optional(&map, "prefix", "fallback"), "fallback");
    }

    #[test]
    fn a_present_optional_flag_overrides_the_default() {
        let map = parse_options(&s(&["--prefix", "oop:check"]), &["prefix"]).unwrap();
        assert_eq!(optional(&map, "prefix", "fallback"), "oop:check");
    }

    #[test]
    fn a_missing_required_flag_is_rejected_by_name() {
        let map = parse_options(&s(&[]), &["template"]).unwrap();
        let e = required(&map, "template").unwrap_err();
        assert!(e.contains("--template"), "{e}");
    }

    #[test]
    fn a_malformed_number_is_rejected_by_name_and_value() {
        let map = parse_options(&s(&["--pot", "abc"]), &["pot"]).unwrap();
        let e = required_u32(&map, "pot").unwrap_err();
        assert!(e.contains("--pot") && e.contains("abc"), "{e}");
    }

    // ---- I3: run-metadata capture parsers (the capture itself runs in the smoke run) ----

    #[test]
    fn the_processor_name_is_read_from_a_reg_query_listing() {
        let listing = "\r\nHKEY_LOCAL_MACHINE\\HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0\r\n    ProcessorNameString    REG_SZ    13th Gen Intel(R) Core(TM) i7-13700K\r\n\r\n";
        assert_eq!(parse_processor_name(listing).as_deref(), Some("13th Gen Intel(R) Core(TM) i7-13700K"));
        assert_eq!(parse_processor_name("ERROR: The system was unable to find the specified registry key or value.\r\n"), None);
        assert_eq!(parse_processor_name("    ProcessorNameString    REG_SZ    \r\n"), None, "an empty name is no name");
    }

    #[test]
    fn rustc_vv_is_summarized_to_version_host_and_llvm() {
        let vv = "rustc 1.95.0 (59807616e 2026-04-14)\nbinary: rustc\ncommit-hash: 59807616e1fa2540724bfbac14d7976d7e4a3860\ncommit-date: 2026-04-14\nhost: x86_64-pc-windows-msvc\nrelease: 1.95.0\nLLVM version: 22.1.2\n";
        assert_eq!(parse_rustc_version(vv).as_deref(), Some("rustc 1.95.0 (59807616e 2026-04-14), host x86_64-pc-windows-msvc, LLVM 22.1.2"));
        assert_eq!(parse_rustc_version(""), None);
        assert_eq!(parse_rustc_version("rustc 1.95.0 (59807616e 2026-04-14)\n"), None, "no host line: not a -vV listing");
    }

    #[test]
    fn bench_date_must_be_an_iso_date() {
        assert!(is_iso_date("2026-09-10"));
        for bad in ["2026-9-10", "2026-09-10x", "20260910", "2026/09/10", "", "2026-13-01", "2026-09-32"] {
            assert!(!is_iso_date(bad), "{bad:?}");
        }
    }
}
