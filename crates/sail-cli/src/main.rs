//! `sail-rs`-CLI: OCaml-kompatibles Frontend fuer den Lean-Pfad (Lane 38).
//!
//! Reines Rust, `#![forbid(unsafe_code)]`. Implementiert Flag-Parsing,
//! `--help`/`-v`, Exit-Codes, bounded `--memo-z3`-Config und das
//! `-dprofile`-Timings-Log. Die eigentliche Sail-Elaboration (Parser,
//! Typechecker, Lean-Emitter) landet mit den Lanes 01-12; bis dahin schreibt
//! der Lean-Pfad einen klar markierten Stub und ist per `--dry-run`
//! vollstaendig ohne Ein-/Ausgaben testbar (Drop-in fuer
//! `arm-v9.4-a/Makefile gen_lean`).
#![forbid(unsafe_code)]

mod args;
mod e2e;
mod memo;
mod ooc;
mod profile;

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

/// Erster Buchstabe gross (OCaml: `-o armv9` -> `Armv9/Armv9.lean`).
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
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
fn check_inputs(
    opts: &Options,
    memo: &mut BoundedMemoCache,
    profiler: &mut Profiler,
) -> Result<(), CliError> {
    profiler.phase("parse", || {});
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

/// Lean-Pfad: Ausgabeverzeichnis vorbereiten + Stub schreiben.
/// Die volle Emission folgt mit Lanes 10-12; der Stub ist als solcher markiert.
fn run_lean(opts: &Options, profiler: &mut Profiler) -> Result<(), CliError> {
    let outdir = opts
        .lean_output_dir
        .clone()
        .unwrap_or_else(|| ".".to_string());
    let prefix = opts
        .output_prefix
        .clone()
        .unwrap_or_else(|| "sail_model".to_string());
    let stem = capitalize_first(&prefix);

    profiler.phase("typecheck", || {});
    profiler.phase("rewrite", || {});

    std::fs::create_dir_all(&outdir).map_err(|e| {
        CliError::fatal(format!(
            "sail-rs: error: cannot create output directory '{outdir}': {e}"
        ))
    })?;

    // Beobachtetes OCaml-Verhalten (0.20.3): nicht-leeres Verzeichnis ohne
    // `--lean-force-output` ist kein Fehler; wir warnen und machen weiter,
    // damit der Drop-in nie an Rest-Artefakten scheitert.
    let non_empty = std::fs::read_dir(&outdir)
        .map(|mut entries| entries.any(|e| e.is_ok()))
        .unwrap_or(false);
    if non_empty && !opts.lean_force_output {
        eprintln!(
            "sail-rs: warning: output directory '{outdir}' is non-empty \
             (use --lean-force-output to request a clean directory)"
        );
    }

    let model_dir = std::path::Path::new(&outdir).join(&stem);
    std::fs::create_dir_all(&model_dir).map_err(|e| {
        CliError::fatal(format!(
            "sail-rs: error: cannot create model directory '{}': {e}",
            model_dir.display()
        ))
    })?;
    let lean_file = model_dir.join(format!("{stem}.lean"));
    let mut content = String::new();
    content.push_str(&format!(
        "-- sail-rs Lane-38 frontend stub (OCaml-compat {COMPAT_SAIL_VERSION}).\n\
         -- Full Lean emission lands with Lanes 10-12; this file only proves the\n\
         -- CLI drop-in (`--lean`, `-o {prefix}`, `--lean-output-dir {outdir}`).\n\
         -- Inputs:\n"
    ));
    for input in &opts.inputs {
        content.push_str(&format!("--   {input}\n"));
    }
    content.push_str(&format!(
        "-- Options: line_width={:?} extern_types={:?} imports={:?}\n",
        opts.lean_line_width, opts.lean_extern_types, opts.lean_import_files
    ));
    std::fs::write(&lean_file, content).map_err(|e| {
        CliError::fatal(format!(
            "sail-rs: error: cannot write '{}': {e}",
            lean_file.display()
        ))
    })?;

    profiler.phase("lean-emit", || {});
    eprintln!(
        "sail-rs: warning: lean emission is a frontend stub (Lane 38); \
         full backend follows with Lanes 10-12"
    );
    verbose_log(opts, &format!("wrote {}", lean_file.display()));
    Ok(())
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

    if opts.dry_run {
        run_dry_run(opts, &mut profiler);
        return Ok(());
    }

    check_inputs(opts, &mut _memo, &mut profiler)?;

    if opts.lean {
        run_lean(opts, &mut profiler)?;
    } else if opts.just_check {
        profiler.phase("typecheck", || {});
        profiler.phase("rewrite", || {});
        verbose_log(opts, "check complete");
    } else if !opts.passthrough_bool.is_empty() || !opts.passthrough_value.is_empty() {
        // Fremdes Backend angefordert (z.B. `-c`): akzeptiert, aber nicht
        // implementiert — kein stiller Erfolg mit falschen Artefakten.
        return Err(CliError {
            exit_code: EXIT_ERROR,
            message: "sail-rs: error: only the lean target and --just-check are \
                      implemented by this frontend (other backend flags are accepted \
                      for parsing but cannot emit)"
                .to_string(),
        });
    } else {
        // Wie OCaml ohne Target: pruefen und beenden.
        profiler.phase("typecheck", || {});
        profiler.phase("rewrite", || {});
    }

    profiler.finish_total();
    if opts.dprofile {
        profiler.emit_stderr();
    }
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

#[cfg(test)]
mod tests {
    use super::capitalize_first;

    #[test]
    fn prefix_capitalization_matches_ocaml() {
        assert_eq!(capitalize_first("armv9"), "Armv9");
        assert_eq!(capitalize_first(""), "");
    }
}
