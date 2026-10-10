//! Smoke driver: parse every `.sail` file given on the command line and
//! report definition counts. Used to check robustness against the real
//! Sail test-suite and ARM model without a full build:
//! `cargo run -p sail-parse --example parse_real -- test/lean/*.sail`.

use std::collections::HashMap;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: parse_real <file.sail>...");
        std::process::exit(2);
    }
    let inputs: Vec<(String, String)> = args
        .iter()
        .map(|p| {
            let src = std::fs::read_to_string(p)
                .unwrap_or_else(|e| panic!("cannot read {p}: {e}"));
            (p.clone(), src)
        })
        .collect();
    let refs: Vec<(&str, &str)> =
        inputs.iter().map(|(p, s)| (p.as_str(), s.as_str())).collect();
    let results = sail_parse::parse_files_parallel(&refs);
    let mut failed = 0;
    let mut total_defs = 0usize;
    let mut kinds: HashMap<&str, usize> = HashMap::new();
    for ((path, _), r) in inputs.iter().zip(results.iter()) {
        match r {
            Ok(pf) => {
                total_defs += pf.defs.len();
                for d in &pf.defs {
                    *kinds.entry(d.def.kind()).or_default() += 1;
                }
            }
            Err(e) => {
                failed += 1;
                println!("FAIL {path}: {e}");
            }
        }
    }
    let mut kinds: Vec<_> = kinds.into_iter().collect();
    kinds.sort();
    println!("files={} failed={failed} defs={total_defs} kinds={kinds:?}", args.len());
    if failed > 0 {
        std::process::exit(1);
    }
}
