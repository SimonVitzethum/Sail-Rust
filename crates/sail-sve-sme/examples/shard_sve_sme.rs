//! Shard driver (Lane 32): parse + analyze the real SVE/SME shards and
//! print the `FAILURES_SVE_SME.md` document to stdout.
//!
//! Usage:
//! ```sh
//! cargo run --example shard_sve_sme -- <instrs64_sve.sail> <instrs64_sme.sail> \
//!   > FAILURES_SVE_SME.md
//! ```
//! Peak RSS (`VmHWM`) is reported to stderr and embedded in the document.
//! Reads the inputs read-only; never writes beside stdout.

#![forbid(unsafe_code)]

use sail_sve_sme::{analyze_files_parallel, shard::render_failures_md};

fn peak_rss_kb() -> Option<u64> {
    // Linux-only; `None` elsewhere (documented in the report).
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            return rest.split_whitespace().next()?.parse::<u64>().ok();
        }
    }
    None
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: shard_sve_sme <instrs64_sve.sail> <instrs64_sme.sail>");
        std::process::exit(2);
    }
    let sve_src = std::fs::read_to_string(&args[1]).unwrap_or_else(|e| {
        eprintln!("cannot read {}: {e}", args[1]);
        std::process::exit(1);
    });
    let sme_src = std::fs::read_to_string(&args[2]).unwrap_or_else(|e| {
        eprintln!("cannot read {}: {e}", args[2]);
        std::process::exit(1);
    });

    let inputs = [("instrs64_sve.sail", sve_src.as_str()), ("instrs64_sme.sail", sme_src.as_str())];
    let results = analyze_files_parallel(&inputs);

    let mut reports = Vec::new();
    let mut errors = Vec::new();
    for ((path, _), r) in inputs.iter().zip(results.iter()) {
        match r {
            Ok(rep) => reports.push(rep.clone()),
            Err(e) => errors.push((path.to_string(), e.to_string())),
        }
    }

    let rss = peak_rss_kb();
    let md = render_failures_md(&reports, &errors, rss, "cargo run --example shard_sve_sme");
    print!("{md}");
    eprintln!(
        "shards: {} ok, {} parse errors; peak RSS: {}",
        reports.len(),
        errors.len(),
        rss.map(|k| format!("{k} KiB")).unwrap_or_else(|| "n/a".to_owned())
    );
}
