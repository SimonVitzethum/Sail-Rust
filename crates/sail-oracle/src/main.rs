#![forbid(unsafe_code)]
//! `sail-oracle-diff`: run the OCaml sail oracle over small `*.sail` tests,
//! diff fresh oracle output against Rust-backend output, and emit a Markdown
//! report (CI artifact, red/green via exit code).
//!
//! Rust output convention: `<stem>.rust.lean` next to each `<stem>.sail`
//! (written by future lanes). Until those exist, cases report an honest
//! `Skipped` — never a fake pass.

use sail_oracle::oracle::{
    self, CaseResult, CaseStatus, MissingBackend, OracleConfig, RustBackend, RUST_OUTPUT_SUFFIX,
};
use sail_oracle::report::{build_markdown, ReportMeta};
use std::path::{Path, PathBuf};

fn usage() -> String {
    "\
sail-oracle-diff: differential harness (OCaml sail = oracle)

Options:
  --sail-bin PATH   oracle binary (default: /home/simon/.opam/sail/bin/sail)
  --test-dir DIR    *.sail directory (default: /home/simon/Dokumente/Sail/sail/test/lean)
  --timeout SECS    wall-clock limit per run (default: 120)
  --mem-mb MB       virtual-memory cap per run (default: 2048)
  --max-bytes N     suite size cap; larger files are nightly-only skips (default: 8192)
  --work-dir DIR    scratch dir for oracle runs (default: $TMPDIR/sail-oracle)
  --report-out FILE write Markdown report to FILE (default: print to stdout)
  --only STEM       run only the case with this file stem (repeatable)
  --help            print this help
"
    .to_string()
}

/// First line of `$sail --help` is `Sail <version>`; fall back to `unknown`.
fn oracle_version(sail_binary: &Path) -> String {
    match std::process::Command::new(sail_binary)
        .arg("--help")
        .output()
    {
        Ok(o) => {
            let all = format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            all.lines().next().unwrap_or("unknown").trim().to_string()
        }
        Err(_) => "unknown (cannot run oracle binary)".to_string(),
    }
}

fn rust_output_for(sail_file: &Path) -> PathBuf {
    let s = sail_file.to_string_lossy();
    let stem = s.strip_suffix(".sail").unwrap_or(&s);
    PathBuf::from(format!("{stem}{RUST_OUTPUT_SUFFIX}"))
}

fn main() {
    let mut cfg = OracleConfig::default();
    let mut report_out: Option<String> = None;
    let mut only: Vec<String> = Vec::new();

    let mut args = std::env::args().skip(1).peekable();
    let val = |args: &mut std::iter::Peekable<std::iter::Skip<std::env::Args>>| -> String {
        args.next().unwrap_or_else(|| {
            eprintln!("missing value for option; see --help");
            std::process::exit(2);
        })
    };
    while let Some(a) = args.next() {
        match a.as_str() {
            "--sail-bin" => cfg.sail_binary = PathBuf::from(val(&mut args)),
            "--test-dir" => cfg.test_dir = PathBuf::from(val(&mut args)),
            "--timeout" => {
                cfg.timeout_secs = val(&mut args).parse().unwrap_or_else(|_| {
                    eprintln!("--timeout needs an integer");
                    std::process::exit(2);
                });
            }
            "--mem-mb" => {
                cfg.memory_mb = val(&mut args).parse().unwrap_or_else(|_| {
                    eprintln!("--mem-mb needs an integer");
                    std::process::exit(2);
                });
            }
            "--max-bytes" => {
                cfg.max_file_bytes = val(&mut args).parse().unwrap_or_else(|_| {
                    eprintln!("--max-bytes needs an integer");
                    std::process::exit(2);
                });
            }
            "--work-dir" => cfg.work_dir = PathBuf::from(val(&mut args)),
            "--report-out" => report_out = Some(val(&mut args)),
            "--only" => only.push(val(&mut args)),
            "--help" | "-h" => {
                print!("{}", usage());
                return;
            }
            other => {
                eprintln!("unknown option {other}\n{}", usage());
                std::process::exit(2);
            }
        }
    }

    let backend = MissingBackend;
    let version = oracle_version(&cfg.sail_binary);

    let mut files = match oracle::discover_tests(&cfg.test_dir) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("cannot list {}: {e}", cfg.test_dir.display());
            std::process::exit(2);
        }
    };
    if !only.is_empty() {
        files.retain(|p| {
            p.file_stem()
                .is_some_and(|s| only.iter().any(|o| o == &s.to_string_lossy()))
        });
    }

    let mut results: Vec<CaseResult> = Vec::with_capacity(files.len());
    for sail_file in &files {
        let name = sail_file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| sail_file.to_string_lossy().into_owned());
        let (oracle_norm, self_check) = oracle::run_oracle_case(&cfg, sail_file);
        let result = match (oracle_norm, &self_check.status) {
            // Oracle run itself broke: report that (red).
            (_, CaseStatus::Timeout)
            | (_, CaseStatus::OomSuspected)
            | (_, CaseStatus::OracleFailed)
            | (_, CaseStatus::HarnessError) => self_check,
            // Oracle produced nothing usable but only a skip: keep the skip.
            (None, _) => self_check,
            (Some(oracle_text), _) => {
                let rust_path = rust_output_for(sail_file);
                match std::fs::read_to_string(&rust_path) {
                    Err(_) => CaseResult::skipped(
                        &name,
                        sail_file,
                        format!(
                            "rust output {} not present yet (backend: {})",
                            rust_path.display(),
                            backend.name()
                        ),
                    ),
                    Ok(rust_raw) => {
                        let rust_norm = sail_oracle::normalize::normalize(&rust_raw);
                        match oracle::compare_outputs(&oracle_text, &rust_norm) {
                            None => CaseResult {
                                name: name.clone(),
                                sail_file: sail_file.clone(),
                                status: CaseStatus::Pass,
                                detail: format!(
                                    "rust output matches oracle ({} bytes normalized)",
                                    oracle_text.len()
                                ),
                            },
                            Some(diff) => CaseResult {
                                name: name.clone(),
                                sail_file: sail_file.clone(),
                                status: CaseStatus::Mismatch,
                                detail: diff,
                            },
                        }
                    }
                }
            }
        };
        let tag = match result.status {
            CaseStatus::Pass => "ok  ",
            CaseStatus::Skipped => "skip",
            _ => "FAIL",
        };
        let first = result.detail.lines().next().unwrap_or("");
        eprintln!("{tag} {} — {} {first}", result.name, result.status.reason());
        results.push(result);
    }

    let meta = ReportMeta {
        oracle_binary: cfg.sail_binary.to_string_lossy().into_owned(),
        oracle_version: version,
        test_dir: cfg.test_dir.to_string_lossy().into_owned(),
        timeout_secs: cfg.timeout_secs,
        memory_mb: cfg.memory_mb,
        rust_backend: backend.name().to_string(),
    };
    let md = build_markdown(&results, &meta);
    if let Some(path) = report_out {
        if let Err(e) = std::fs::write(&path, &md) {
            eprintln!("cannot write report {path}: {e}");
            std::process::exit(2);
        }
        let failed = results.iter().filter(|r| r.is_failure()).count();
        eprintln!(
            "report -> {path} ({} cases, {} failing)",
            results.len(),
            failed
        );
    } else {
        print!("{md}");
    }
    if results.iter().any(|r| r.is_failure()) {
        std::process::exit(1);
    }
}
