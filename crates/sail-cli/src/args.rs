//! OCaml-kompatibles Flag-Parsing fuer die `sail-rs`-CLI (Lane 38).
//!
//! Kompatibilitaetsregeln (beobachtet an OCaml-Sail 0.20.3):
//! - Ein oder zwei fuehrende `-` sind aequivalent (`-lean` == `--lean`).
//! - `-` und `_` im Flag-Namen sind aequivalent (`-memo_z3` == `--memo-z3`).
//! - Flag-Namen sind case-sensitiv (`-o` != `-O`).
//! - `--flag=wert` ist aequivalent zu `--flag wert`; boolsche Flags nehmen
//!   keinen Wert (`--lean=x` ist ein Usage-Fehler).
//! - Aneinandergehaengte Kurzform (`-oarmv9`) wird NICHT akzeptiert.
#![forbid(unsafe_code)]

/// OCaml-Sail-Version, zu der diese CLI kompatibel ist.
pub const COMPAT_SAIL_VERSION: &str = "0.20.3";

/// Exit-Codes, angelehnt an OCaml-Sail (beobachtet):
/// - `0`: Erfolg (auch `--help`, `-v`, Dry-Run-Plan).
/// - `1`: Usage-/Sail-Fehler (unbekanntes Flag, fehlender/ungueltiger Wert,
///   `--require-version` verletzt, Typfehler).
/// - `2`: I/O-fatal (Eingabedatei fehlt/nicht lesbar, Ausgabeverzeichnis-Konflikt).
pub const EXIT_OK: i32 = 0;
pub const EXIT_ERROR: i32 = 1;
pub const EXIT_FATAL: i32 = 2;

/// Standardwert fuer `--memo-z3-path` (wie OCaml-Sail).
pub const DEFAULT_MEMO_Z3_PATH: &str = "sail_smt_cache";
/// Bound fuer den `--memo-z3`-Cache: maximale Eintragszahl (sail-rs-Bound,
/// OCaml memoiziert unbeschraenkt; siehe `memo.rs`).
pub const DEFAULT_MEMO_Z3_MAX_ENTRIES: usize = 10_000;
/// Bound fuer den `--memo-z3`-Cache: maximale ungefaehre Bytegrenze.
pub const DEFAULT_MEMO_Z3_MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    pub exit_code: i32,
    pub message: String,
}

impl CliError {
    fn usage(msg: impl Into<String>) -> Self {
        Self {
            exit_code: EXIT_ERROR,
            message: msg.into(),
        }
    }

    pub fn fatal(msg: impl Into<String>) -> Self {
        Self {
            exit_code: EXIT_FATAL,
            message: msg.into(),
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// Geparste CLI-Optionen. Felder ohne `Option`/`Vec` entsprechen direkt den
/// OCaml-Flags; `passthrough_*` nimmt übrige OCaml-Backend-Flags entgegen
/// (akzeptiert, aber von diesem Lean-Frontend ignoriert), damit fremde
/// `SAIL_FLAGS` keinen Drop-in-Bruch verursachen.
#[derive(Debug, Clone, Default)]
pub struct Options {
    // --- Lean-Target (strikt implementiert) ---
    pub lean: bool,
    pub lean_output_dir: Option<String>,
    pub lean_force_output: bool,
    pub lean_single_file: bool,
    pub lean_matchbv: bool,
    pub lean_noncomputable: bool,
    pub lean_real_numbers: bool,
    pub lean_executable: bool,
    pub lean_line_width: Option<u32>,
    pub lean_extern_types: Vec<String>,
    pub lean_import_files: Vec<String>,
    pub lean_noncomputable_functions: Vec<String>,
    pub lean_partial_functions: Vec<String>,
    pub lean_non_beq_types: Vec<String>,
    pub lean_lib_path: Option<String>,
    pub lean_lib_repo: Option<String>,
    pub lean_lib_rev: Option<String>,
    // --- Globale Optionen ---
    pub output_prefix: Option<String>,
    pub memo_z3: bool,
    pub memo_z3_path: String,
    pub memo_z3_max_entries: usize,
    pub memo_z3_max_bytes: u64,
    pub dprofile: bool,
    pub verbose: u32,
    pub just_check: bool,
    pub no_core: bool,
    pub grouped_regstate: bool,
    pub no_warn: bool,
    pub warn_error: bool,
    pub defines: Vec<String>,
    pub require_version: Option<String>,
    pub show_dir: bool,
    pub print_version: bool,
    pub print_help: bool,
    pub dry_run: bool,
    // --- E2E-Tiny-Arm-Treiber (Lane 116 verdrahtet Lane-45-`e2e.rs`; rein
    // additiv, keine bestehenden Flags angefasst; endgültige Flag-Parität
    // bleibt Lane-38-Hoheit) ---
    pub e2e: bool,
    pub e2e_out_dir: Option<String>,
    pub e2e_module: Option<String>,
    pub inputs: Vec<String>,
    // --- Akzeptiert-und-ignoriert (andere OCaml-Backends) ---
    pub passthrough_bool: Vec<String>,
    pub passthrough_value: Vec<(String, String)>,
}

impl Options {
    pub fn new() -> Self {
        Self {
            memo_z3: true, // wie OCaml: Memoisierung ist Default
            memo_z3_path: DEFAULT_MEMO_Z3_PATH.to_string(),
            memo_z3_max_entries: DEFAULT_MEMO_Z3_MAX_ENTRIES,
            memo_z3_max_bytes: DEFAULT_MEMO_Z3_MAX_BYTES,
            ..Self::default()
        }
    }
}

/// Normalisiert einen Flag-Namen: 1-2 fuehrende `-` entfernen, `_` zu `-`.
/// Case-sensitiv (kein Lowercasing: `-o` != `-O`).
fn normalize_flag(raw: &str) -> Option<&str> {
    let stripped = raw.strip_prefix("--").or_else(|| raw.strip_prefix('-'))?;
    if stripped.is_empty() || stripped.starts_with('-') {
        return None;
    }
    Some(stripped)
}

fn canonical(raw: &str) -> Option<String> {
    normalize_flag(raw).map(|s| s.replace('_', "-"))
}

fn is_bool_passthrough(name: &str) -> bool {
    matches!(
        name,
        // Globale/Debug-/Mono-/SMT-Passthroughs
        "i" | "interact-custom" | "all-modules" | "list-files" | "list-files-separated"
        | "fmt" | "slice-instantiation-types" | "no-warn" | "all-warnings"
        | "strict-var" | "strict-bitvector" | "strict-exponentials"
        | "no-color" | "string-literal-type" | "non-lexical-flow"
        | "no-lexp-bounds-check" | "auto-mono" | "mono-rewrites"
        | "smt-linearize" | "Oconstant-fold" | "Oaarch64-fast" | "Ofast-undefined"
        | "const-prop-mutrec"
        | "ddump-project-depgraph" | "ddump-project-depgraph-reduced"
        | "ddump-project-depgraph-skip-deps" | "ddump-project-depgraph-skip-reqs"
        | "ddump-initial-ast" | "ddump-tc-ast" | "ddump-side-effect"
        | "dsmt-verbose" | "dallow-internal" | "dno-error-filenames"
        | "unroll-loops" | "dmono-all-split-errors" | "dmono-continue"
        | "dmono-limit" | "dsequential"
        | "infer-effects" | "undefined-gen"
        | "explain-all-variables" | "explain-constraints" | "explain-all-overloads"
        | "explain-verbose"
        // C/C++
        | "c" | "c-build" | "c-no-mangle" | "c-no-main" | "c-no-rts" | "c-no-lib"
        | "c-generate-header" | "c-specialize" | "c-assert-to-exception"
        | "c-fold-unit" | "O" | "Ofixed-int" | "Ofixed-bits" | "static" | "cpp"
        // Rocq/Coq
        | "coq" | "rocq" | "rocq-generate-extern-types" | "rocq-record-update"
        | "rocq-undef-axioms" | "rocq-minimal-eq-dec"
        | "rocq-separate-interface-file" | "rocq-generic-value"
        // SMT/Doc/HTML
        | "smt" | "smt-auto" | "smt-ignore-overflow" | "smt-propagate-vars"
        | "smt-disable-specialization"
        | "doc" | "doc-embed-with-location" | "doc-compact" | "html"
        // SystemVerilog (boolsch)
        | "sv" | "sv-global-prefix" | "sv-lines" | "sv-comb" | "sv-no-write-flush"
        | "sv-inregs" | "sv-outregs" | "sv-no-unions" | "sv-no-strings"
        | "sv-no-packed" | "sv-no-assertions" | "sv-never-pack-unions"
        | "sv-assert-to-exception" | "sv-skip-cyclic" | "sv-nomem"
        | "sv-disable-optimizations" | "sv-no-assert-fatal" | "sv-assert-as-property"
        // OCaml/Lem/LaTeX/Sail-Backend
        | "ocaml" | "ocaml-nobuild" | "ocaml-trace"
        | "output-sail" | "latex" | "latex-full-valspecs"
        | "lem" | "lem-split-files" | "lem-sequential" | "lem-mwords"
    )
}

fn is_value_passthrough(name: &str) -> bool {
    matches!(
        name,
        "is" | "iout"
            | "project"
            | "variable"
            | "instantiate"
            | "tool"
            | "config"
            | "sail-config"
            | "output-schema"
            | "plugin"
            | "have-feature"
            | "mono-split"
            | "splice"
            | "smt-solver"
            | "dtc-verbose"
            | "dcallgraph"
            | "ddump-rewrite-ast"
            | "dmono-analysis"
            | "dbacktrace"
            | "unroll-loops-max-iter"
            | "c-include"
            | "c-header-include"
            | "c-prefix"
            | "c-extra-params"
            | "c-extra-args"
            | "c-preserve"
            | "c-preserve-type"
            | "c-coverage"
            | "cpp-class-name"
            | "cpp-namespace"
            | "cpp-derive-from"
            | "rocq-output-dir"
            | "rocq-lib"
            | "rocq-alt-modules"
            | "rocq-alt-modules2"
            | "rocq-extern-type"
            | "rocq-isla"
            | "rocq-lib-style"
            | "drocq-debug-on"
            | "smt-auto-solver"
            | "smt-int-size"
            | "smt-bits-size"
            | "smt-vector-size"
            | "smt-include"
            | "doc-format"
            | "doc-file"
            | "doc-embed"
            | "doc-bundle"
            | "html-css"
            | "html-link-prefix"
            | "sv-output-dir"
            | "sv-include"
            | "sv-toplevel"
            | "sv-verilate"
            | "sv-verilate-args"
            | "sv-verilate-cflags"
            | "sv-verilate-ldflags"
            | "sv-verilate-jobs"
            | "sv-recursion-depth"
            | "sv-int-size"
            | "sv-bits-size"
            | "sv-unreachable"
            | "sv-fun2wires"
            | "sv-specialize"
            | "sv-dpi"
            | "sv-verilate-link-sail-runtime"
            | "ocaml-build-dir"
            | "ocaml-generators"
            | "latex-prefix"
            | "latex-abbrevs"
            | "lem-output-dir"
            | "isa-output-dir"
            | "lem-lib"
            | "lem-extern-type"
    )
}

fn parse_u32(flag: &str, value: &str) -> Result<u32, CliError> {
    value.parse::<u32>().map_err(|_| {
        CliError::usage(format!(
            "sail-rs: option '--{flag}' requires an integer argument, got '{value}'"
        ))
    })
}

fn parse_usize(flag: &str, value: &str) -> Result<usize, CliError> {
    value.parse::<usize>().map_err(|_| {
        CliError::usage(format!(
            "sail-rs: option '--{flag}' requires an integer argument, got '{value}'"
        ))
    })
}

fn parse_u64(flag: &str, value: &str) -> Result<u64, CliError> {
    value.parse::<u64>().map_err(|_| {
        CliError::usage(format!(
            "sail-rs: option '--{flag}' requires an integer argument, got '{value}'"
        ))
    })
}

/// Parst `argv` (inkl. `argv[0]`). Gibt bei `--help`/`-v` entsprechende
/// Marker in `Options` zurueck; der Aufrufer druckt und beendet mit 0.
pub fn parse_args(argv: &[String]) -> Result<Options, CliError> {
    let mut opts = Options::new();
    let mut iter = argv.iter().skip(1).peekable();
    let mut only_inputs = false;

    while let Some(arg) = iter.next() {
        if only_inputs {
            opts.inputs.push(arg.clone());
            continue;
        }
        if arg == "--" {
            only_inputs = true;
            continue;
        }
        if !arg.starts_with('-') || arg == "-" {
            opts.inputs.push(arg.clone());
            continue;
        }

        // `--flag=wert`-Form aufspalten.
        let (flag_part, inline_value) = match arg.split_once('=') {
            Some((f, v)) => (f, Some(v)),
            None => (arg.as_str(), None),
        };
        let Some(name) = canonical(flag_part) else {
            return Err(CliError::usage(format!(
                "sail-rs: unknown option '{arg}'\nUse 'sail-rs --help' for a list of available arguments."
            )));
        };

        // Holt den Wert entweder aus `--flag=wert` oder dem naechsten argv.
        let mut take_value = |flag_display: &str| -> Result<String, CliError> {
            if let Some(v) = inline_value {
                return Ok(v.to_string());
            }
            iter.next().cloned().ok_or_else(|| {
                CliError::usage(format!(
                    "sail-rs: option '{flag_display}' requires an argument\nUse 'sail-rs --help' for a list of available arguments."
                ))
            })
        };
        // Boolsche Flags duerfen kein `=wert` tragen (wie OCaml: Usage-Fehler).
        let check_no_inline = |flag_display: &str| -> Result<(), CliError> {
            if inline_value.is_some() {
                return Err(CliError::usage(format!(
                    "sail-rs: option '{flag_display}' takes no argument\nUse 'sail-rs --help' for a list of available arguments."
                )));
            }
            Ok(())
        };

        match name.as_str() {
            // --- Hilfe/Version ---
            "h" | "help" => {
                check_no_inline(arg)?;
                opts.print_help = true;
            }
            "v" | "version" => {
                check_no_inline(arg)?;
                opts.print_version = true;
            }
            // --- Lean-Target ---
            "lean" => {
                check_no_inline(arg)?;
                opts.lean = true;
            }
            "lean-output-dir" => {
                opts.lean_output_dir = Some(take_value(arg)?);
            }
            "lean-force-output" => {
                check_no_inline(arg)?;
                opts.lean_force_output = true;
            }
            "lean-single-file" => {
                check_no_inline(arg)?;
                opts.lean_single_file = true;
            }
            "lean-matchbv" => {
                check_no_inline(arg)?;
                opts.lean_matchbv = true;
            }
            "lean-noncomputable" => {
                check_no_inline(arg)?;
                opts.lean_noncomputable = true;
            }
            "lean-real-numbers" => {
                check_no_inline(arg)?;
                opts.lean_real_numbers = true;
            }
            "lean-executable" => {
                check_no_inline(arg)?;
                opts.lean_executable = true;
            }
            "lean-line-width" => {
                let v = take_value(arg)?;
                opts.lean_line_width = Some(parse_u32("lean-line-width", &v)?);
            }
            "lean-extern-type" => {
                opts.lean_extern_types.push(take_value(arg)?);
            }
            "lean-import-file" => {
                opts.lean_import_files.push(take_value(arg)?);
            }
            "lean-noncomputable-function" => {
                opts.lean_noncomputable_functions.push(take_value(arg)?);
            }
            "lean-partial-function" => {
                opts.lean_partial_functions.push(take_value(arg)?);
            }
            "lean-non-beq-type" => {
                opts.lean_non_beq_types.push(take_value(arg)?);
            }
            "lean-lib-path" => {
                opts.lean_lib_path = Some(take_value(arg)?);
            }
            "lean-lib-repo" => {
                opts.lean_lib_repo = Some(take_value(arg)?);
            }
            "lean-lib-rev" => {
                opts.lean_lib_rev = Some(take_value(arg)?);
            }
            // --- Globale Optionen ---
            "o" => {
                opts.output_prefix = Some(take_value(arg)?);
            }
            "dir" => {
                check_no_inline(arg)?;
                opts.show_dir = true;
            }
            "memo-z3" => {
                check_no_inline(arg)?;
                opts.memo_z3 = true;
            }
            "no-memo-z3" => {
                check_no_inline(arg)?;
                opts.memo_z3 = false;
            }
            "memo-z3-path" => {
                opts.memo_z3_path = take_value(arg)?;
            }
            "memo-z3-max-entries" => {
                let v = take_value(arg)?;
                let n = parse_usize("memo-z3-max-entries", &v)?;
                if n == 0 {
                    return Err(CliError::usage(
                        "sail-rs: option '--memo-z3-max-entries' must be > 0",
                    ));
                }
                opts.memo_z3_max_entries = n;
            }
            "memo-z3-max-bytes" => {
                let v = take_value(arg)?;
                opts.memo_z3_max_bytes = parse_u64("memo-z3-max-bytes", &v)?;
            }
            "dprofile" => {
                check_no_inline(arg)?;
                opts.dprofile = true;
            }
            "verbose" => {
                let v = take_value(arg)?;
                opts.verbose = parse_u32("verbose", &v)?;
            }
            "just-check" => {
                check_no_inline(arg)?;
                opts.just_check = true;
            }
            "no-core" => {
                check_no_inline(arg)?;
                opts.no_core = true;
            }
            "grouped-regstate" => {
                check_no_inline(arg)?;
                opts.grouped_regstate = true;
            }
            "no-warn" => {
                check_no_inline(arg)?;
                opts.no_warn = true;
            }
            "warn-error" => {
                check_no_inline(arg)?;
                opts.warn_error = true;
            }
            "D" => {
                opts.defines.push(take_value(arg)?);
            }
            "require-version" => {
                opts.require_version = Some(take_value(arg)?);
            }
            "dry-run" => {
                check_no_inline(arg)?;
                opts.dry_run = true;
            }
            "e2e" => {
                check_no_inline(arg)?;
                opts.e2e = true;
            }
            "out-dir" => {
                opts.e2e_out_dir = Some(take_value(arg)?);
            }
            "e2e-module" => {
                opts.e2e_module = Some(take_value(arg)?);
            }
            _ if is_bool_passthrough(&name) => {
                check_no_inline(arg)?;
                opts.passthrough_bool.push(name);
            }
            _ if is_value_passthrough(&name) => {
                let v = take_value(arg)?;
                opts.passthrough_value.push((name, v));
            }
            _ => {
                return Err(CliError::usage(format!(
                    "sail-rs: unknown option '{arg}'\nUse 'sail-rs --help' for a list of available arguments."
                )));
            }
        }
    }

    Ok(opts)
}

/// Vergleicht zwei punktseparierte Versionen numerisch pro Komponente.
/// Gibt `Ordering` von `have` relativ zu `want` zurueck.
pub fn compare_versions(have: &str, want: &str) -> std::cmp::Ordering {
    let parse = |v: &str| {
        v.split('.')
            .map(|c| {
                c.chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
            })
            .map(|c| c.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let (mut h, mut w) = (parse(have), parse(want));
    let n = h.len().max(w.len());
    h.resize(n, 0);
    w.resize(n, 0);
    h.cmp(&w)
}

/// `--help`-Text im OCaml-Sail-Stil. Die Lean-Sektion ist wortgleich mit
/// `sail --help` (0.20.3); uebrige Backends werden als akzeptiert-ignoriert
/// dokumentiert; sail-rs-Erweiterungen und Exit-Codes sind am Ende erklaert.
pub fn help_text(pkg_version: &str) -> String {
    format!(
        "sail-rs {pkg_version} (OCaml-compatible frontend for Sail {COMPAT_SAIL_VERSION})\n\
usage: sail-rs <options> <file1.sail> ... <fileN.sail>\n\
\n\
  -o <prefix>                               select output filename prefix\n\
  --dir                                     show current Sail library directory\n\
  --just-check                              terminate immediately after typechecking\n\
  --no-core                                 do not use any core definitions\n\
  --memo-z3                                 memoize calls to z3, improving performance when typechecking repeatedly (default)\n\
  --no-memo-z3                              do not memoize calls to z3\n\
  --memo-z3-path path                       to cache z3 results (default 'sail_smt_cache')\n\
  -D <symbol>                               define a symbol for the preprocessor, as $define does in the source code\n\
  --no-warn                                 do not print warnings\n\
  --warn-error                              promote warnings to errors, if below the required version\n\
  --grouped-regstate                        group registers with same type together in generated register state record\n\
  --dprofile                                (debug) provide basic profiling information for rewriting passes within Sail\n\
  -v                                        print version\n\
  --version                                 print version\n\
  --require-version <min_version>           exit with non-zero status if Sail version requirement is not met\n\
  --verbose <verbosity>                     produce verbose output\n\
  -h                                        display this list of options. Also available as -help or --help\n\
  -help                                     display this list of options\n\
  --help                                    display this list of options\n\
\n\
Options for target lean\n\
  --lean                                    invoke the Sail lean target\n\
  --lean-output-dir <directory>             set a custom directory to output generated Lean\n\
  --lean-force-output                       removes the content of the output directory if it is non-empty\n\
  --lean-single-file                        puts the entire output in a single .lean file\n\
  --lean-matchbv                            use matchbv in the Lean output\n\
  --lean-line-width                         maximum line length of the generated Lean code\n\
  --lean-noncomputable                      add a 'noncomputable section' at the beginning of the output\n\
  --lean-real-numbers                       enable real numbers (in particular, depend on mathlib)\n\
  --lean-extern-type <typename>             do not generate a definition for the type\n\
  --lean-import-file <file>                 import this file in the generated model\n\
  --lean-noncomputable-function <func-name> do not generate executable code for this function\n\
  --lean-partial-function <func-name>       disable the totality check for this function\n\
  --lean-non-beq-type <func-name>           disable deriving a BEq instance for this type\n\
  --lean-executable                         generate an executable if there is a main function in the Sail program\n\
  --lean-lib-path                           (local) path of the Lean support library, for development\n\
  --lean-lib-repo                           url of the git repository of the Lean support library\n\
  --lean-lib-rev                            revision of the Lean support library\n\
\n\
Other OCaml targets (c, cpp, coq, rocq, smt, doc, html, systemverilog, ocaml,\n\
latex, lem): flags are accepted for drop-in compatibility and otherwise\n\
ignored by this Lean frontend. Single-dash and double-dash as well as '-' and\n\
'_' in flag names are equivalent, as in OCaml Sail. '--flag=value' works.\n\
\n\
sail-rs extensions\n\
  --dry-run                                 validate arguments and print the execution plan without reading\n\
                                            inputs or writing outputs (also via env SAIL_RS_DRY_RUN=1)\n\
  --e2e                                     run the Tiny-Arm E2E driver (Lane-45 sketch in src/e2e.rs):\n\
                                            parse one input, emit stub + Lake skeleton to --out-dir,\n\
                                            report the Fail list (exit 1) or I/O problems (exit 2)\n\
  --out-dir <directory>                     output directory for --e2e (default target/sail-e2e/SailTinyArm)\n\
  --e2e-module <name>                       Lean module name for --e2e (default SailTinyArm)\n\
  --memo-z3-max-entries <n>                 bound the in-memory memo-z3 cache to <n> entries (default 10000).\n\
                                            OCaml-Sail memoizes unboundedly; sail-rs always stays bounded.\n\
  --memo-z3-max-bytes <n>                   bound the in-memory memo-z3 cache to roughly <n> bytes (default 67108864)\n\
\n\
-dprofile log format (stderr, Lane-14-style timings log, one line per phase):\n\
  [profile] phase=\"<name>\" wall_s=<secs> rss_peak_mb=<mb> smt_calls=<n> smt_s=<secs>\n\
  (OCaml-Sail prints 'Profiled <phase>: ...' instead; the key=value form above\n\
  is the sail-rs equivalent and additionally reports peak RSS per phase.)\n\
\n\
Exit codes\n\
  0  success (including --help, -v and --dry-run)\n\
  1  usage or Sail error (unknown flag, missing/invalid value, --require-version\n\
     not met, type error)\n\
  2  I/O fatal (input file missing/unreadable, output directory conflict)\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &[&str]) -> Vec<String> {
        std::iter::once("sail-rs".to_string())
            .chain(words.iter().map(|w| w.to_string()))
            .collect()
    }

    #[test]
    fn dash_underscore_and_single_double_dash_are_equivalent() {
        for flag in [
            "--lean",
            "-lean",
            "--memo-z3",
            "-memo_z3",
            "--dprofile",
            "-dprofile",
        ] {
            let opts = parse_args(&argv(&[flag, "--just-check"])).unwrap();
            match flag {
                f if f.contains("lean") && !f.contains("memo") => assert!(opts.lean, "{flag}"),
                f if f.contains("memo") => assert!(opts.memo_z3, "{flag}"),
                _ => assert!(opts.dprofile, "{flag}"),
            }
        }
    }

    #[test]
    fn lean_gen_lean_argv_parses() {
        // Nahezu wortgleich mit dem `gen_lean`-Aufruf aus arm-v9.4-a/Makefile.
        let opts = parse_args(&argv(&[
            "-dprofile",
            "-verbose",
            "1",
            "-memo_z3",
            "-lean",
            "--lean-force-output",
            "-lean_output_dir",
            "lean",
            "-o",
            "armv9",
            "-undefined_gen",
            "-memo-z3",
            "-lean-noncomputable",
            "-lean-line-width",
            "100000",
            "-lean-force-output",
            "-lean-import-file",
            "lean/ArmExtras.lean",
            "src/prelude.sail",
            "src/termination_measures.sail",
        ]))
        .unwrap();
        assert!(opts.dprofile);
        assert_eq!(opts.verbose, 1);
        assert!(opts.memo_z3);
        assert!(opts.lean);
        assert!(opts.lean_force_output);
        assert_eq!(opts.lean_output_dir.as_deref(), Some("lean"));
        assert_eq!(opts.output_prefix.as_deref(), Some("armv9"));
        assert_eq!(opts.lean_line_width, Some(100_000));
        assert!(opts.lean_noncomputable);
        assert_eq!(
            opts.lean_import_files,
            vec!["lean/ArmExtras.lean".to_string()]
        );
        assert!(opts.passthrough_bool.contains(&"undefined-gen".to_string()));
        assert_eq!(opts.inputs.len(), 2);
    }

    #[test]
    fn equals_form_works_for_value_flags() {
        let opts = parse_args(&argv(&[
            "--lean-output-dir=lean",
            "-o=armv9",
            "--verbose=2",
        ]))
        .unwrap();
        assert_eq!(opts.lean_output_dir.as_deref(), Some("lean"));
        assert_eq!(opts.output_prefix.as_deref(), Some("armv9"));
        assert_eq!(opts.verbose, 2);
    }

    #[test]
    fn bool_flag_with_equals_is_usage_error() {
        let err = parse_args(&argv(&["--lean=true"])).unwrap_err();
        assert_eq!(err.exit_code, EXIT_ERROR);
    }

    #[test]
    fn unknown_flag_is_exit_1() {
        let err = parse_args(&argv(&["--bogus-flag"])).unwrap_err();
        assert_eq!(err.exit_code, EXIT_ERROR);
        assert!(err.message.contains("unknown option"));
    }

    #[test]
    fn missing_value_is_exit_1() {
        let err = parse_args(&argv(&["-o"])).unwrap_err();
        assert_eq!(err.exit_code, EXIT_ERROR);
    }

    #[test]
    fn invalid_integer_is_exit_1() {
        let err = parse_args(&argv(&["--lean-line-width", "weit"])).unwrap_err();
        assert_eq!(err.exit_code, EXIT_ERROR);
    }

    #[test]
    fn joined_short_form_is_rejected_like_ocaml() {
        // OCaml-Sail akzeptiert `-oarmv9` nicht; wir auch nicht.
        let err = parse_args(&argv(&["-oarmv9"])).unwrap_err();
        assert_eq!(err.exit_code, EXIT_ERROR);
    }

    #[test]
    fn case_sensitive_o_vs_big_o() {
        let opts = parse_args(&argv(&["-O", "-o", "armv9"])).unwrap();
        assert!(opts.passthrough_bool.contains(&"O".to_string()));
        assert_eq!(opts.output_prefix.as_deref(), Some("armv9"));
    }

    #[test]
    fn repeatable_lean_flags_accumulate() {
        let opts = parse_args(&argv(&[
            "--lean-extern-type",
            "a",
            "--lean-extern-type",
            "b",
            "--lean-import-file=f.lean",
        ]))
        .unwrap();
        assert_eq!(
            opts.lean_extern_types,
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(opts.lean_import_files, vec!["f.lean".to_string()]);
    }

    #[test]
    fn no_memo_z3_disables_default() {
        let opts = parse_args(&argv(&["--no-memo-z3"])).unwrap();
        assert!(!opts.memo_z3);
        let opts = parse_args(&argv(&[])).unwrap();
        assert!(opts.memo_z3); // Default wie OCaml: an
    }

    #[test]
    fn memo_bounds_must_be_positive() {
        let err = parse_args(&argv(&["--memo-z3-max-entries", "0"])).unwrap_err();
        assert_eq!(err.exit_code, EXIT_ERROR);
        let opts = parse_args(&argv(&["--memo-z3-max-entries", "512"])).unwrap();
        assert_eq!(opts.memo_z3_max_entries, 512);
    }

    #[test]
    fn double_dash_ends_flag_parsing() {
        let opts = parse_args(&argv(&["--", "--lean"])).unwrap();
        assert!(!opts.lean);
        assert_eq!(opts.inputs, vec!["--lean".to_string()]);
    }

    #[test]
    fn version_comparison_is_numeric() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_versions("0.20.3", "0.20.3"), Equal);
        assert_eq!(compare_versions("0.20.3", "999.0.0"), Less);
        assert_eq!(compare_versions("0.20.3", "0.9.0"), Greater);
        assert_eq!(compare_versions("0.20.3", "0.20.10"), Less);
    }

    #[test]
    fn help_text_covers_required_flags() {
        let help = help_text("0.1.0");
        for needle in [
            "--lean",
            "--lean-output-dir",
            "--lean-force-output",
            "-o <prefix>",
            "--memo-z3",
            "--dprofile",
            "--lean-line-width",
            "--lean-extern-type",
            "-v ",
            "usage: sail-rs <options>",
            "Exit codes",
            "[profile]",
        ] {
            assert!(help.contains(needle), "missing: {needle}");
        }
    }

    #[test]
    fn e2e_flags_parsen_mit_out_dir_und_modul() {
        let opts = parse_args(&argv(&[
            "--e2e",
            "--out-dir",
            "out",
            "--e2e-module",
            "Tiny",
            "in.sail",
        ]))
        .unwrap();
        assert!(opts.e2e);
        assert_eq!(opts.e2e_out_dir.as_deref(), Some("out"));
        assert_eq!(opts.e2e_module.as_deref(), Some("Tiny"));
        assert_eq!(opts.inputs, vec!["in.sail".to_string()]);
    }

    #[test]
    fn e2e_bool_mit_gleich_ist_usage_fehler() {
        let err = parse_args(&argv(&["--e2e=true"])).unwrap_err();
        assert_eq!(err.exit_code, EXIT_ERROR);
    }

    #[test]
    fn out_dir_underscore_form_ist_aequivalent() {
        let opts = parse_args(&argv(&["--out_dir", "o", "--e2e"])).unwrap();
        assert_eq!(opts.e2e_out_dir.as_deref(), Some("o"));
        assert!(opts.e2e);
    }
}
