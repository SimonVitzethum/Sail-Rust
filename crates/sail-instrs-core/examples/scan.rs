//! Einmal-Analyse einer `.sail`-Datei als Strom (kein Full-Build).
//!
//! Gebrauch: `cargo run -p sail-instrs-core --example scan -- <datei.sail>`
//! Gibt Shard-Plan, Top-5-Funktionen, Dispatch-Fanout und Kantenstatistik aus.
#![forbid(unsafe_code)]

use std::env;
use std::fs::File;
use std::io::{BufReader, Result};

use sail_instrs_core::dispatch::DispatchGraph;
use sail_instrs_core::measure::{decode_fanout, largest};
use sail_instrs_core::plan::FilePlan;
use sail_instrs_core::scanner::scan_headers;

fn main() -> Result<()> {
    let path = env::args().nth(1).expect("usage: scan <datei.sail>");
    let open = || -> Result<BufReader<File>> { Ok(BufReader::new(File::open(&path)?)) };

    let headers = scan_headers(open()?)?;
    let mut plan = FilePlan::default();
    for h in &headers {
        plan.add(h);
    }
    println!("file={path}");
    println!("defs={} exec={} decode={} dispatch={} helper={}",
        plan.total(), plan.exec, plan.decode, plan.dispatch, plan.helper);

    println!("top5_by_lines:");
    for f in largest(open()?, 5)? {
        println!("  lines={:>4} arms={:>4} ifs={:>3} {} @L{}",
            f.lines, f.arms, f.ifs, f.name, f.line);
    }

    println!("dispatch_fanout:");
    for (target, n) in decode_fanout(open()?)? {
        println!("  clauses={:>5} {target}", n);
    }

    let g = DispatchGraph::build(open()?)?;
    println!("decode_edges={} clause_edges={}", g.decode_edges.len(), g.clause_edges.len());
    println!("decode_multi_execute={:?}", g.decode_without_single_execute());
    println!("see_violations={:?}", g.see_monotonic_violations());
    Ok(())
}
