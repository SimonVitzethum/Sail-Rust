//! CLI: `sail-sysregs-shard <datei.sail> [--defs-per-block N]`
//!
//! Streamt genau eine Datei durch den Shard-Runner und druckt den Report
//! (Zeit + Peak-RSS als `VmHWM`). Exit-Code != 0 bei Split-/Block-Fehler.

#![forbid(unsafe_code)]

use sail_sysregs_shard::{run_shard, ShardConfig};
use std::process::ExitCode;

fn usage() -> ! {
    eprintln!("usage: sail-sysregs-shard <datei.sail> [--defs-per-block N]");
    std::process::exit(2);
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        usage();
    }
    let mut path: Option<&str> = None;
    let mut defs_per_block = 64usize;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--defs-per-block" => {
                i += 1;
                if i >= args.len() {
                    usage();
                }
                defs_per_block = args[i].parse().unwrap_or_else(|_| usage());
            }
            other => {
                if path.is_some() {
                    usage();
                }
                path = Some(other);
            }
        }
        i += 1;
    }
    let path = match path {
        Some(p) => p,
        None => usage(),
    };
    let config = match (ShardConfig { defs_per_block }).validated() {
        Ok(c) => c,
        Err(m) => {
            eprintln!("error: {m}");
            return ExitCode::from(2);
        }
    };
    match run_shard(path, config) {
        Ok(r) => {
            println!("file:               {}", r.path);
            println!("bytes:              {}", r.total_bytes);
            println!("lines:              {}", r.total_lines);
            println!("defs:               {}", r.total_defs);
            println!("defs_per_block:     {}", r.defs_per_block);
            println!("blocks:             {}", r.block_count());
            println!("max_block_bytes:    {}", r.max_block_bytes());
            println!("combined_fingerprint: {:016x}", r.combined_fingerprint());
            println!("elapsed_ms:         {}", r.elapsed_ms);
            match r.peak_rss_kb {
                Some(kb) => println!("peak_rss_kb:        {kb} ({:.2} MiB)", kb as f64 / 1024.0),
                None => println!("peak_rss_kb:        n/a (no /proc/self/status)"),
            }
            let growth = r
                .blocks
                .iter()
                .filter_map(|b| b.rss_kb_after)
                .max()
                .unwrap_or(0);
            println!("max_sampled_rss_kb: {growth}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}
