//! Rust Sail CLI. Ordinary compilation runs real preprocessing and bounded
//! grammar/AST conversion. Missing checking or emission stages return errors;
//! a frontend placeholder must never be reported as a successful build.
#![forbid(unsafe_code)]

mod args;
mod e2e;
mod frontend;
mod memo;
mod lean_stage;
mod ooc;
mod profile;
mod rewrite_stage;
mod typecheck;

use std::io::Write;

use args::{
    compare_versions, help_text, parse_args, CliError, Options, COMPAT_SAIL_VERSION, EXIT_ERROR,
    EXIT_OK,
};
use memo::{BoundedMemoCache, BoundedMemoConfig};
use profile::Profiler;

fn version_string() -> String {
    format!(
        "sail-rs {} (OCaml-compatible frontend for Sail {})",
        env!("CARGO_PKG_VERSION"),
        COMPAT_SAIL_VERSION
    )
}

/// Prueft `--require-version` gegen die Kompatibilitaetsversion.
/// Exit 1 mit OCaml-aehnlicher Meldung, wenn unerfuellt.
fn check_require_version(opts: &Options) -> Result<(), CliError> {
    if let Some(want) = &opts.require_version {
        if compare_versions(COMPAT_SAIL_VERSION, want) == std::cmp::Ordering::Less {
            return Err(CliError {
                exit_code: EXIT_ERROR,
                message: format!(
                    "Sail version {COMPAT_SAIL_VERSION} is older than requested version {want}"
                ),
            });
        }
    }
    Ok(())
}

fn verbose_log(opts: &Options, msg: &str) {
    if opts.verbose > 0 {
        eprintln!("sail-rs: [verbose={}] {msg}", opts.verbose);
    }
}

/// Dry-Run-Plan auf stdout; keine Ein-/Ausgaben, Exit 0.
/// Entspricht dem `gen_lean`-Aufruf aus `arm-v9.4-a/Makefile` (Drop-in).
fn run_dry_run(opts: &Options, profiler: &mut Profiler) {
    println!("sail-rs: dry-run plan (no inputs read, no outputs written)");
    println!(
        "  target={} outdir={} prefix={} files=[{}]",
        if opts.lean { "lean" } else { "check" },
        opts.lean_output_dir.as_deref().unwrap_or("(default)"),
        opts.output_prefix.as_deref().unwrap_or("(default)"),
        opts.inputs.join(", "),
    );
    println!(
        "  memo-z3={} path={} max_entries={} max_bytes={} profile={} verbose={}",
        if opts.memo_z3 { "bounded" } else { "off" },
        opts.memo_z3_path,
        opts.memo_z3_max_entries,
        opts.memo_z3_max_bytes,
        if opts.dprofile { "on" } else { "off" },
        opts.verbose,
    );
    for phase in ["parse", "typecheck", "rewrite", "lean-emit"] {
        profiler.simulated(phase);
    }
    profiler.finish_total();
    if opts.dprofile {
        profiler.emit_stderr();
    }
}

/// Prueft, dass alle Eingaben existieren (fehlend -> Exit 2 wie OCaml-Sail).
/// Stat-Ergebnisse werden im bounded Memo-Cache abgelegt, sodass doppelt
/// genannte Eingaben kein zweites I/O kosten (echte Solver-Memoisierung
/// folgt mit Lane 06).
fn check_inputs(opts: &Options, memo: &mut BoundedMemoCache) -> Result<(), CliError> {
    if opts.inputs.is_empty() {
        return Err(CliError {
            exit_code: EXIT_ERROR,
            message:
                "sail-rs: no input files\nUse 'sail-rs --help' for a list of available arguments."
                    .to_string(),
        });
    }
    for input in &opts.inputs {
        verbose_log(opts, &format!("input={input}"));
        // Memo-Key: "stat:<pfad>", Value: Dateigroesse (LE-u64, 8 Bytes).
        let key = format!("stat:{input}").into_bytes();
        let known_good = memo.get(&key).is_some();
        if !known_good {
            let size = std::fs::metadata(input)
                .map(|m| m.len())
                .unwrap_or(u64::MAX);
            if size == u64::MAX || !std::path::Path::new(input).is_file() {
                return Err(CliError::fatal(format!(
                    "sail-rs: error: cannot open input file '{input}': No such file or directory"
                )));
            }
            memo.insert(key, size.to_le_bytes().to_vec());
        }
    }
    Ok(())
}

/// Run the real disk-backed frontend before rejecting missing compiler stages.
/// No generated Lean project is published until every required stage exists.
fn run_compiler(opts: &Options, profiler: &mut Profiler) -> Result<(), CliError> {
    let mut effective_options = opts.clone();
    let frontend = profiler.phase("frontend", || frontend::run(&mut effective_options))?;
    let summary = &frontend.summary;
    eprintln!("sail-rs: frontend converted {} definitions from {} files using {} workers; {} AST bytes spooled",
        summary.definitions, summary.files, summary.workers, summary.ast_bytes);
    let checked = profiler.phase("typecheck", || typecheck::run(&frontend))?;
    eprintln!("sail-rs: typechecked {} definitions and {} expressions using {} workers", checked.definitions, checked.expressions, summary.workers);
    let rewritten = profiler.phase("rewrite", || rewrite_stage::run(&frontend, &checked))?;
    eprintln!("sail-rs: rewrote {} definitions through {} passes ({})",
        rewritten.definitions, rewritten.passes.len(), rewritten.passes.join(", "));
    rewritten.require_lean_emit()?;
    Err(CliError {
        exit_code: EXIT_ERROR,
        message: "sail-rs: compilation incomplete: full Sail typechecking, rewrite integration and Lean body emission are not implemented; frontend conversion is not a successful model build".into(),
    })
}

/// E2E-Tiny-Arm-Treiber (Lane 116 verdrahtet den Lane-45-Einstieg `e2e::run`
/// ans CLI): `--e2e <input.sail> --out-dir <dir> [--e2e-module <name>]`.
/// Kehrt nie mit stillem Erfolg über Lücken hinweg zurück — offene Bodies
/// stehen als Fail-Liste im Fehler (Exit 1), I/O-Probleme als fatal (Exit 2).
/// Die Flag-Schreibweise ist vorläufig; endgültige Flag-Parität bleibt
/// Lane-38-Hoheit.
fn run_e2e(
    opts: &Options,
    memo: &mut BoundedMemoCache,
    profiler: &mut Profiler,
) -> Result<(), CliError> {
    if opts.inputs.len() != 1 {
        return Err(CliError {
            exit_code: EXIT_ERROR,
            message: "sail-rs: --e2e expects exactly one input file\nUse 'sail-rs --help' for a list of available arguments."
                .to_string(),
        });
    }
    let args = e2e::E2eArgs::with_input(
        opts.inputs[0].clone(),
        opts.e2e_out_dir.clone(),
        opts.e2e_module.clone(),
    );
    if opts.dry_run {
        println!("sail-rs: dry-run plan (no inputs read, no outputs written)");
        println!(
            "  target=e2e input={} outdir={} module={}",
            args.input, args.out_dir, args.module,
        );
        profiler.simulated("parse");
        profiler.simulated("lean-emit");
        profiler.finish_total();
        if opts.dprofile {
            profiler.emit_stderr();
        }
        return Ok(());
    }
    check_inputs(opts, memo)?;
    verbose_log(
        opts,
        &format!("e2e input={} outdir={}", args.input, args.out_dir),
    );
    profiler.phase("typecheck", || {});
    let outcome = e2e::run(&args);
    profiler.phase("lean-emit", || {});
    profiler.finish_total();
    if opts.dprofile {
        profiler.emit_stderr();
    }
    println!(
        "sail-rs: e2e {} -> {} (module {}): {} file(s), {} fail(s), ooc sealed={} evicted={}",
        args.input,
        args.out_dir,
        args.module,
        outcome.emitted.len(),
        outcome.fails.len(),
        outcome.ooc.sealed,
        outcome.ooc.evicted,
    );
    for f in &outcome.emitted {
        println!("sail-rs: e2e emitted {f}");
    }
    for f in &outcome.fails {
        println!("sail-rs: e2e {f}");
    }
    match e2e::exit_code_for(&outcome) {
        0 => Ok(()),
        2 => Err(CliError::fatal(format!(
            "sail-rs: e2e failed with {} I/O fail(s), see report above",
            outcome.fails.len()
        ))),
        _ => Err(CliError {
            exit_code: EXIT_ERROR,
            message: format!(
                "sail-rs: e2e incomplete: {} fail(s), see report above",
                outcome.fails.len()
            ),
        }),
    }
}

fn run(opts: &Options) -> Result<(), CliError> {
    if opts.print_help {
        print!("{}", help_text(env!("CARGO_PKG_VERSION")));
        return Ok(());
    }
    if opts.print_version {
        println!("{}", version_string());
        return Ok(());
    }
    if opts.show_dir {
        let libdir = std::env::var("SAIL_RS_LIBDIR").unwrap_or_else(|_| ".".to_string());
        println!("{libdir}");
        return Ok(());
    }
    check_require_version(opts)?;

    let mut profiler = Profiler::new();
    profiler.begin_total();

    // Bounded Memo-Config validieren (Speicher-Schranke gilt ab Lane 06
    // auch fuer echte Solver-Aufrufe; hier bereits enforced, siehe memo.rs).
    let memo_config = if opts.memo_z3 {
        BoundedMemoConfig::new(
            true,
            opts.memo_z3_path.clone(),
            opts.memo_z3_max_entries,
            opts.memo_z3_max_bytes,
        )
    } else {
        BoundedMemoConfig::disabled()
    };
    let mut _memo = if memo_config.enabled {
        BoundedMemoCache::new(memo_config)
    } else {
        BoundedMemoCache::disabled()
    };
    if let Some(cfg) = _memo.config() {
        verbose_log(
            opts,
            &format!(
                "memo-z3=bounded path={} max_entries={} max_bytes={}",
                cfg.path, cfg.max_entries, cfg.max_bytes
            ),
        );
    } else {
        verbose_log(opts, "memo-z3=off");
    }

    if opts.e2e {
        return run_e2e(opts, &mut _memo, &mut profiler);
    }

    if opts.dry_run {
        run_dry_run(opts, &mut profiler);
        return Ok(());
    }

    check_inputs(opts, &mut _memo)?;

    let compilation = if opts.lean || opts.just_check {
        run_compiler(opts, &mut profiler)
    } else if !opts.passthrough_bool.is_empty() || !opts.passthrough_value.is_empty() {
        Err(CliError {
            exit_code: EXIT_ERROR,
            message: "sail-rs: error: requested backend is not implemented".into(),
        })
    } else {
        run_compiler(opts, &mut profiler)
    };

    profiler.finish_total();
    if opts.dprofile {
        profiler.emit_stderr();
    }
    compilation?;
    let (entries, hits, misses, evictions) = _memo.stats();
    verbose_log(
        opts,
        &format!(
            "done phases={} memo_entries={entries} hits={hits} misses={misses} evictions={evictions}",
            profiler.samples().len()
        ),
    );
    Ok(())
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let mut opts = match parse_args(&argv) {
        Ok(opts) => opts,
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "{}", e.message);
            std::process::exit(e.exit_code);
        }
    };
    // Env-Variante von `--dry-run` fuer Makefile-Drop-in ohne Arg-Aenderung.
    if std::env::var("SAIL_RS_DRY_RUN").is_ok() {
        opts.dry_run = true;
    }
    match run(&opts) {
        Ok(()) => std::process::exit(EXIT_OK),
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "{}", e.message);
            std::process::exit(e.exit_code.max(EXIT_ERROR));
        }
    }
}
