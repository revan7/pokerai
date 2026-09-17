use bench::{gen_spots, materialize};
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (code, message) = match args.get(1).map(String::as_str) {
        Some("materialize") => match cmd_materialize(&args[2..]) { Ok(json) => { println!("{json}"); (0, None) } Err(e) => (2, Some(e)) },
        Some("gen-spots") => match cmd_gen_spots(&args[2..]) { Ok(code) => (code, None), Err(e) => (2, Some(e)) },
        _ => (1, Some("usage: bench materialize --template ID --pot P --eff E [--prefix ...] | bench gen-spots [--source r8] [--out DIR] | bench run ... (Task 30)".to_string())),
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
}
