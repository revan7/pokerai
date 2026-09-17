use bench::{gen_spots, materialize};

fn arg(args: &[String], name: &str) -> Option<String> { args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned()) }

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let code = match args.get(1).map(String::as_str) {
        Some("materialize") => {
            let r = materialize::run(&arg(&args, "--template").unwrap_or_default(), arg(&args, "--pot").and_then(|v| v.parse().ok()).unwrap_or(0),
                arg(&args, "--eff").and_then(|v| v.parse().ok()).unwrap_or(0), &arg(&args, "--prefix").unwrap_or_default());
            match r { Ok(json) => { println!("{json}"); 0 } Err(e) => { eprintln!("{e}"); 2 } }
        }
        Some("gen-spots") => {
            let out = std::path::PathBuf::from(arg(&args, "--out").unwrap_or_else(|| "bench/spots".into()));
            let source = arg(&args, "--source").unwrap_or_else(|| "r8".into());
            let mut code = 0;
            for suite in ["river_std", "river_min", "turn_std", "turn_min"] {
                match gen_spots::generate(suite, &source).and_then(|s| s.save(&out.join(format!("{suite}.json")))) { Ok(()) => println!("wrote {suite}"), Err(e) => { eprintln!("{e}"); code = 2 } }
            }
            code
        }
        _ => { eprintln!("usage: bench materialize --template ID --pot P --eff E [--prefix ...] | bench gen-spots [--source r8] [--out DIR] | bench run ... (Task 30)"); 1 }
    };
    std::process::exit(code);
}
