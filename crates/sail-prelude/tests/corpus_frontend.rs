#![forbid(unsafe_code)]
//! Korpus-Gate-Treiber (Lane 81): alle `sail/lib`-Dateien durchs Frontend.
//!
//! The fragment gate preprocesses conditionals before bridging each file.
//! Includes stay explicit: this per-file test is not a whole-program check.
//! und resolvt mit `sail_resolve::resolve` (Lane 03). Fehler → katalogisiert
//! als [`sail_prelude::corpus::FileReport`] (Stufe + Meldung); deferred
//! Bridge-Reste sind KEIN Fehler, nur Lücke mit Owner aus
//! [`sail_prelude::corpus::CORPUS_FILES`] (siehe `LIB_COVERAGE.md`).
//!
//! Scope:
//! - Immer: die 26 vendored Dateien aus `sail_prelude::PRELUDE_FILES`.
//! - Optional: volle 59 via `SAIL_LIB_DIR` (zeigt auf `sail/lib/`-Checkout,
//!   inkl. `core/`, `float/`, `concurrency_interface/`). Fehlt die Variable
//!   oder das Verzeichnis, läuft nur der vendored Teil (lauter Skip).

use std::path::{Path, PathBuf};

use sail_prelude::corpus::{CORPUS_FILES, CorpusSummary, FileReport, FileStatus, GateStage};
use sail_prelude::PRELUDE_FILES;

/// Eine Gate-Eingabe: relativer Korpus-Pfad + Quelltext.
struct Input {
    rel: String,
    src: String,
}

fn sail_lib_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("sail-lib")
}

fn collect_sail(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot list {}: {e}", dir.display()));
    let mut entries: Vec<_> = entries.map(|e| e.unwrap().path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_sail(&path, out);
        } else if path.extension().map(|e| e == "sail").unwrap_or(false) {
            out.push(path);
        }
    }
}

/// Eine Datei durch parse → bridge → resolve treiben.
fn run_one(rel: &str, src: &str) -> FileReport {
    let mut parsed = sail_parse::ParsedFile { path: rel.into(), defs: Vec::new() };
    let mut preprocessor = sail_parse::preprocess::Preprocessor::new(Some("lean".into()));
    // Library fragments need the same implicit builtins as model inputs.
    // Provide a default order explicitly so vector.sail's guard is exercised.
    preprocessor.process("defaults", "default Order dec\n", |_, _| Ok(None), |_, _, _| Ok(())).unwrap();
    preprocessor.process(sail_prelude::CORE_LIBRARY.name,
        sail_prelude::CORE_LIBRARY.contents, |_, _| Ok(None),
        |_, _, def| { parsed.defs.push(def); Ok(()) }).unwrap();
    let preprocessed = preprocessor.process(rel, src, |_, _| Ok(None),
        |_, _, def| { parsed.defs.push(def); Ok(()) });
    let bridged = match preprocessed.map(|()| sail_parse::bridge_parsed(&parsed)) {
        Ok(b) => b,
        Err(e) => {
            return FileReport {
                rel: rel.to_owned(),
                status: FileStatus::ParseError,
                failed_stage: Some(GateStage::Parse),
                message: Some(e.to_string()),
                bridged_defs: 0,
                deferred: 0,
                resolve_errors: 0,
                resolve_warnings: 0,
            };
        }
    };
    let bridged_defs = bridged.ast.defs.len();
    let deferred = bridged.deferred.len();
    let resolved = sail_resolve::resolve::resolve(&bridged.ast, &bridged.interner);
    let resolve_errors = resolved.errors.len();
    let resolve_warnings = resolved.warnings.len();
    if !resolved.is_ok() {
        let first = resolved.errors.first().map(|e| e.to_string()).or_else(|| {
            if bridged.deferred.is_empty() {
                None
            } else {
                Some(format!("resolve failed with {} deferred defs", bridged.deferred.len()))
            }
        });
        return FileReport {
            rel: rel.to_owned(),
            status: FileStatus::ResolveError,
            failed_stage: Some(GateStage::Resolve),
            message: first,
            bridged_defs,
            deferred,
            resolve_errors,
            resolve_warnings,
        };
    }
    FileReport {
        rel: rel.to_owned(),
        status: if deferred > 0 {
            FileStatus::DeferredGap
        } else {
            FileStatus::Pass
        },
        failed_stage: None,
        message: None,
        bridged_defs,
        deferred,
        resolve_errors,
        resolve_warnings,
    }
}

fn report_failures(reports: &[FileReport]) -> String {
    reports
        .iter()
        .filter(|r| !r.is_gate_ok())
        .map(|r| {
            format!(
                "{}: {} at {}: {}",
                r.rel,
                r.status.as_str(),
                r.failed_stage.map(|s| s.as_str()).unwrap_or("?"),
                r.message.as_deref().unwrap_or("(no message)")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Vendored Gate: alle 26 `PRELUDE_FILES` müssen parse- + resolve-ok sein
/// (deferred-Rest erlaubt, mit Owner in `LIB_COVERAGE.md`).
#[test]
fn corpus_vendored_frontend_gate() {
    assert_eq!(PRELUDE_FILES.len(), 26, "vendored inventory must be 26 files");
    let inputs: Vec<Input> = PRELUDE_FILES
        .iter()
        .map(|f| Input {
            rel: f.name.to_owned(),
            src: f.contents.to_owned(),
        })
        .collect();
    let reports: Vec<FileReport> =
        inputs.iter().map(|i| run_one(&i.rel, &i.src)).collect();
    let summary = CorpusSummary::summarize(&reports);
    eprintln!("{}", summary.one_line());
    for r in &reports {
        eprintln!(
            "  {}: {} (bridged={} deferred={} resolve-errors={} warnings={})",
            r.rel,
            r.status.as_str(),
            r.bridged_defs,
            r.deferred,
            r.resolve_errors,
            r.resolve_warnings
        );
    }
    assert!(
        reports.iter().all(|r| r.is_gate_ok()),
        "vendored gate failures ({} files):\n{}",
        reports.len(),
        report_failures(&reports)
    );
}

/// Voller Gate: 59 Dateien aus `SAIL_LIB_DIR` (`sail/lib/`-Checkout).
/// Ohne Variable/Verzeichnis: lauter Skip (nur vendored läuft, siehe oben).
#[test]
fn corpus_full_lib_frontend_gate() {
    let ext_dir: Option<PathBuf> = std::env::var("SAIL_LIB_DIR").ok().map(PathBuf::from);
    let Some(dir) = ext_dir else {
        eprintln!("SKIP full-lib gate: SAIL_LIB_DIR not set (vendored gate covers 26 files)");
        return;
    };
    if !dir.is_dir() {
        eprintln!("SKIP full-lib gate: SAIL_LIB_DIR={} is not a directory", dir.display());
        return;
    }
    let mut files = Vec::new();
    collect_sail(&dir, &mut files);
    assert!(!files.is_empty(), "no .sail files under {}", dir.display());
    assert_eq!(
        files.len(),
        CORPUS_FILES.len(),
        "sail/lib inventory drift: found {} .sail files, model expects {}",
        files.len(),
        CORPUS_FILES.len()
    );
    let mut reports = Vec::new();
    for path in &files {
        let rel = path
            .strip_prefix(&dir)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let src = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        reports.push(run_one(&rel, &src));
    }
    let summary = CorpusSummary::summarize(&reports);
    eprintln!("{}", summary.one_line());
    for r in &reports {
        eprintln!(
            "  {}: {} (bridged={} deferred={} resolve-errors={} warnings={} owner={})",
            r.rel,
            r.status.as_str(),
            r.bridged_defs,
            r.deferred,
            r.resolve_errors,
            r.resolve_warnings,
            sail_prelude::corpus::owner_for(&r.rel)
        );
    }
    // Vendored Konsistenz: externe Top-Level-Dateien == eingebettete Bytes.
    let vendored_dir = sail_lib_dir();
    if vendored_dir.is_dir() {
        for f in PRELUDE_FILES {
            let ext = std::fs::read_to_string(vendored_dir.join(f.name))
                .unwrap_or_else(|e| panic!("cannot read vendored {}: {e}", f.name));
            assert_eq!(ext, f.contents, "vendored {} drifts from sail-lib", f.name);
        }
    }
    assert!(
        reports.iter().all(|r| r.is_gate_ok()),
        "full-lib gate failures:\n{}",
        report_failures(&reports)
    );
}
