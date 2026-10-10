#![forbid(unsafe_code)]
//! Oracle side of the differential harness: run OCaml `sail` on small
//! `*.sail` tests with per-run timeout + RAM limit, normalize its Lean
//! output, and compare it against the Rust backend output.
//!
//! Verified oracle invocation (Sail 0.20.3, Oct 2026):
//! ```text
//! mkdir -p <case-outdir>
//! sail --lean-matchbv <file.sail> --lean --lean-single-file \
//!     --lean-output-dir <case-outdir>
//! ```
//! Lean output lands in `<case-outdir>/out/*.lean` (plus lake scaffolding,
//! which is ignored). No `--lean-lib-path` / lake build is needed to produce
//! the files, so the default suite stays offline.
//!
//! Limits are enforced without any `unsafe` code: the child runs under
//! `bash -c 'ulimit -v <kb>; exec timeout <s>s ...'`, i.e. virtual-memory
//! cap via `ulimit` plus wall-clock kill via coreutils `timeout`.
//! Exit-code reading (the `timeout(1)` convention):
//! - `124` → wall-clock timeout,
//! - `137` (SIGKILL) or OOM-killer traces on stderr → OOM suspected,
//! - anything else non-zero → oracle failure (stderr tail kept).

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Default extra flags, mirroring upstream `test/suites/lean.py`
/// (non-runnable `test/lean` directory).
pub const DEFAULT_EXTRA_ARGS: &[&str] = &["--lean-matchbv", "--lean", "--lean-single-file"];
/// Default wall-clock limit per oracle run (ARM cases are *not* in the
/// default suite; small tests finish in seconds).
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// Default virtual-memory cap per oracle run.
pub const DEFAULT_MEMORY_MB: u64 = 2048;
/// Default suite cap: files larger than this are skipped (nightly-only).
/// Covers `SailTinyArm*.sail`, `map_tactics.sail`, `riscv_duopod.sail`.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 8192;
/// File suffix of checked-in oracle expectations next to each `.sail` file.
pub const EXPECTED_SUFFIX: &str = ".expected.lean";
/// File suffix for Rust-backend output dropped next to each `.sail` file by
/// future lanes (picked up automatically once present).
pub const RUST_OUTPUT_SUFFIX: &str = ".rust.lean";

#[derive(Debug, Clone)]
pub struct OracleConfig {
    pub sail_binary: PathBuf,
    pub test_dir: PathBuf,
    pub extra_args: Vec<String>,
    pub timeout_secs: u64,
    pub memory_mb: u64,
    pub max_file_bytes: u64,
    pub work_dir: PathBuf,
}

impl Default for OracleConfig {
    fn default() -> Self {
        Self {
            sail_binary: PathBuf::from("/home/simon/.opam/sail/bin/sail"),
            test_dir: PathBuf::from("/home/simon/Dokumente/Sail/sail/test/lean"),
            extra_args: DEFAULT_EXTRA_ARGS.iter().map(|s| s.to_string()).collect(),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            memory_mb: DEFAULT_MEMORY_MB,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            work_dir: std::env::temp_dir().join("sail-oracle"),
        }
    }
}

impl OracleConfig {
    /// Virtual-memory cap in KiB for `ulimit -v`.
    pub fn memory_kb(&self) -> u64 {
        self.memory_mb.saturating_mul(1024)
    }
}

/// Outcome of one differential test case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaseStatus {
    /// Oracle and Rust output are identical (after normalization).
    Pass,
    /// Outputs differ; `detail` holds the unified diff.
    Mismatch,
    /// The oracle itself failed; `detail` holds exit + stderr tail.
    OracleFailed,
    /// Wall-clock limit hit (`timeout(1)` exit 124).
    Timeout,
    /// SIGKILL / OOM-killer traces; `detail` holds the evidence.
    OomSuspected,
    /// Not run; `detail` is the reason (too large, rust output missing…).
    Skipped,
    /// The harness (not the oracle) broke; `detail` is the message.
    HarnessError,
}

impl CaseStatus {
    /// Human-readable short reason for the failing-list (`file + reason`).
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Mismatch => "output mismatch vs oracle",
            Self::OracleFailed => "oracle run failed",
            Self::Timeout => "oracle timeout",
            Self::OomSuspected => "suspected oracle OOM",
            Self::Skipped => "skipped",
            Self::HarnessError => "harness error",
        }
    }

    /// Whether this status fails the CI job (red).
    pub fn is_failure(&self) -> bool {
        matches!(
            self,
            Self::Mismatch
                | Self::OracleFailed
                | Self::Timeout
                | Self::OomSuspected
                | Self::HarnessError
        )
    }
}

#[derive(Debug, Clone)]
pub struct CaseResult {
    /// Test name = `.sail` file stem (e.g. `trivial`).
    pub name: String,
    pub sail_file: PathBuf,
    pub status: CaseStatus,
    /// Reason / stderr tail / diff depending on `status`.
    pub detail: String,
}

impl CaseResult {
    pub fn skipped(name: &str, sail_file: &Path, reason: impl Into<String>) -> Self {
        Self {
            name: name.to_string(),
            sail_file: sail_file.to_path_buf(),
            status: CaseStatus::Skipped,
            detail: reason.into(),
        }
    }

    pub fn is_failure(&self) -> bool {
        self.status.is_failure()
    }
}

/// Sorted list of `*.sail` files directly inside `test_dir`.
pub fn discover_tests(test_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(test_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "sail"))
        .collect();
    out.sort();
    Ok(out)
}

/// Rust backend plug-in point. Later lanes implement this for real; until
/// then [`MissingBackend`] makes every case report an honest skip instead of
/// a fake pass.
pub trait RustBackend {
    fn name(&self) -> &str;
    fn emit_lean(&self, sail_file: &Path, out_file: &Path) -> Result<(), String>;
}

/// Placeholder until a later lane wires the real Rust compiler.
pub struct MissingBackend;

impl RustBackend for MissingBackend {
    fn name(&self) -> &str {
        "missing (no Rust sail backend yet)"
    }

    fn emit_lean(&self, _sail_file: &Path, _out_file: &Path) -> Result<(), String> {
        Err("rust sail backend not implemented yet (later lane)".to_string())
    }
}

/// Compare normalized oracle output vs normalized Rust output.
/// Returns `Some(diff)` on mismatch, `None` when equal.
pub fn compare_outputs(oracle_normalized: &str, rust_normalized: &str) -> Option<String> {
    let diff = crate::diff::unified_diff(oracle_normalized, rust_normalized, 3);
    if diff.is_empty() {
        None
    } else {
        Some(diff)
    }
}

/// Keep the last `n` lines of `text` (for stderr tails in reports).
pub fn last_lines(text: &str, n: usize) -> String {
    if n == 0 {
        return String::new();
    }
    let lines: Vec<&str> = text.lines().collect();
    let skip = lines.len().saturating_sub(n);
    lines[skip..].join("\n")
}

/// Classify a finished oracle child process (pure, unit-tested).
///
/// `code` is the process exit code (`None` = killed by signal without code).
/// `stderr` is used only for the OOM-killer heuristic.
pub fn classify_exit(code: Option<i32>, stderr: &str) -> (CaseStatus, String) {
    let oom_trace = stderr.contains("Killed")
        || stderr.contains("Out of memory")
        || stderr.contains("Cannot allocate memory")
        || stderr.contains("memory exhausted");
    match code {
        Some(124) => (
            CaseStatus::Timeout,
            "killed by timeout(1), exit 124".to_string(),
        ),
        Some(137) | None if oom_trace => (
            CaseStatus::OomSuspected,
            format!(
                "SIGKILL/OOM evidence; stderr tail:\n{}",
                last_lines(stderr, 10)
            ),
        ),
        Some(137) | None => (
            CaseStatus::OomSuspected,
            "exit 137/SIGKILL without OOM trace (treat as suspected OOM)".to_string(),
        ),
        Some(0) => (CaseStatus::Pass, String::new()),
        Some(c) => (
            CaseStatus::OracleFailed,
            format!("exit {c}; stderr tail:\n{}", last_lines(stderr, 15)),
        ),
    }
}

/// Build the `(program, args)` used to run one oracle case under limits.
///
/// Runs `bash -c 'ulimit -v <kb>; exec timeout <s>s <sail> <args>'` with
/// `cwd` = `scratch_dir` (keeps `sail_smt_cache` out of the repo).
/// Pure constructor, unit-tested; see [`run_oracle_case`] for execution.
pub fn build_oracle_command(
    cfg: &OracleConfig,
    sail_file: &Path,
    case_out_dir: &Path,
) -> (String, Vec<String>) {
    let script = format!(
        "ulimit -v {}; exec timeout {}s \"$@\"",
        cfg.memory_kb(),
        cfg.timeout_secs
    );
    let mut args = vec!["-c".to_string(), script, "sail-oracle".to_string()];
    args.push(cfg.sail_binary.to_string_lossy().into_owned());
    args.extend(cfg.extra_args.iter().cloned());
    args.push(sail_file.to_string_lossy().into_owned());
    args.push("--lean-output-dir".to_string());
    args.push(case_out_dir.to_string_lossy().into_owned());
    ("bash".to_string(), args)
}

/// Unique scratch directory for one case run.
fn unique_dir(base: &Path, stem: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    base.join(format!("{stem}-{nanos}-{}", std::process::id()))
}

/// Collect oracle Lean output in exactly the upstream self-check shape
/// (`test/suites/lean.py`): `Out/Defs.lean`, then a blank line,
/// `XXXXXXXXX`, another blank line, then `Out.lean`:
///
/// ```text
/// cat <out>/out/Out/Defs.lean > output
/// echo >> output; echo "XXXXXXXXX" >> output; echo >> output
/// cat <out>/out/Out.lean >> output
/// diff output <basename>.expected.lean
/// ```
///
/// Lake scaffolding (`lakefile.toml`, …) is ignored, same as upstream.
/// Returns an error description when the two files are not both present.
pub fn collect_oracle_output(case_out_dir: &Path) -> Result<String, String> {
    let lean_root = case_out_dir.join("out");
    let defs_path = lean_root.join("Out").join("Defs.lean");
    let top_path = lean_root.join("Out.lean");
    let mut missing = Vec::new();
    if !defs_path.is_file() {
        missing.push(defs_path.display().to_string());
    }
    if !top_path.is_file() {
        missing.push(top_path.display().to_string());
    }
    if !missing.is_empty() {
        return Err(format!("oracle output missing: {}", missing.join(", ")));
    }
    let read = |p: &Path| {
        std::fs::read_to_string(p).map_err(|e| format!("cannot read {}: {e}", p.display()))
    };
    let defs = read(&defs_path)?;
    let top = read(&top_path)?;
    Ok(format!("{defs}\nXXXXXXXXX\n\n{top}"))
}

/// Stage one case's inputs into `scratch` and return the **bare** input file
/// name to pass to the oracle.
///
/// The oracle embeds the input path as given on the command line into the
/// generated Lean (`assert false "Pattern match failure at <path>:…"`), and
/// loads sibling `<stem>.json` configs from the working directory — both
/// exactly like upstream `test/suites/lean.py`, which runs with
/// `cwd=testdir` and a bare filename. So we copy `<stem>.sail` (plus
/// `<stem>.json` when present) into `scratch` and invoke the oracle with
/// just the file name.
pub fn stage_input(sail_file: &Path, scratch: &Path) -> io::Result<String> {
    let file_name = sail_file
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    std::fs::copy(sail_file, scratch.join(&file_name))?;
    let stem = file_name.strip_suffix(".sail").unwrap_or(&file_name);
    let json_name = format!("{stem}.json");
    if let Some(parent) = sail_file.parent() {
        let sibling = parent.join(&json_name);
        if sibling.is_file() {
            std::fs::copy(&sibling, scratch.join(&json_name))?;
        }
    }
    Ok(file_name)
}

/// Run the oracle for one `.sail` file and diff against the checked-in
/// `<stem>.expected.lean` (environment self-check).
///
/// Returns `(oracle_normalized, CaseResult-for-self-check)`. The self-check
/// result is `Pass` when fresh oracle output matches the checked-in file,
/// `OracleFailed` when it drifts, `Skipped` when no expected file exists,
/// or `Timeout`/`OomSuspected`/`HarnessError` when the run itself broke.
pub fn run_oracle_case(cfg: &OracleConfig, sail_file: &Path) -> (Option<String>, CaseResult) {
    let name = sail_file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| sail_file.to_string_lossy().into_owned());
    let fail = |status: CaseStatus, detail: String| {
        (
            None,
            CaseResult {
                name: name.clone(),
                sail_file: sail_file.to_path_buf(),
                status,
                detail,
            },
        )
    };

    let size = match std::fs::metadata(sail_file) {
        Ok(m) => m.len(),
        Err(e) => return fail(CaseStatus::HarnessError, format!("cannot stat input: {e}")),
    };
    if size > cfg.max_file_bytes {
        return (
            None,
            CaseResult::skipped(
                &name,
                sail_file,
                format!(
                    "{size} bytes > {} byte suite cap (nightly-only, e.g. ARM)",
                    cfg.max_file_bytes
                ),
            ),
        );
    }

    let scratch = unique_dir(&cfg.work_dir, &name);
    let case_out = scratch.join("case-out");
    if let Err(e) = std::fs::create_dir_all(&case_out) {
        return fail(
            CaseStatus::HarnessError,
            format!("cannot create scratch {}: {e}", case_out.display()),
        );
    }
    // Bare filename + cwd=scratch: keeps generated paths (`….sail:line`)
    // identical to upstream and finds sibling `<stem>.json` configs.
    let staged = match stage_input(sail_file, &scratch) {
        Ok(f) => f,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&scratch);
            return fail(CaseStatus::HarnessError, format!("cannot stage input: {e}"));
        }
    };
    // Relative output dir: nothing absolute leaks into oracle output.
    let (prog, args) = build_oracle_command(cfg, Path::new(&staged), Path::new("case-out"));
    let output = match Command::new(&prog)
        .args(&args)
        .current_dir(&scratch)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            return fail(
                CaseStatus::HarnessError,
                format!("cannot spawn oracle: {e}"),
            )
        }
    };
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let (status, mut detail) = classify_exit(output.status.code(), &stderr);
    if status != CaseStatus::Pass {
        let _ = std::fs::remove_dir_all(&scratch);
        return (
            None,
            CaseResult {
                name,
                sail_file: sail_file.to_path_buf(),
                status,
                detail,
            },
        );
    }
    // status == Pass so far => collect + normalize + self-check vs expected.
    let raw = match collect_oracle_output(&case_out) {
        Ok(r) => r,
        Err(e) => {
            detail = format!(
                "oracle exited 0 but {e}; stderr tail:\n{}",
                last_lines(&stderr, 10)
            );
            let _ = std::fs::remove_dir_all(&scratch);
            return (
                None,
                CaseResult {
                    name,
                    sail_file: sail_file.to_path_buf(),
                    status: CaseStatus::OracleFailed,
                    detail,
                },
            );
        }
    };
    let redactions = [scratch.to_string_lossy().into_owned()];
    let red_refs: Vec<&str> = redactions.iter().map(|s| s.as_str()).collect();
    let normalized = crate::normalize::normalize_with(&raw, &red_refs);
    let _ = std::fs::remove_dir_all(&scratch);

    // Self-check against checked-in expectation (if any).
    let expected_path = {
        let s = sail_file.to_string_lossy();
        let stem = s.strip_suffix(".sail").unwrap_or(&s);
        PathBuf::from(format!("{stem}{EXPECTED_SUFFIX}"))
    };
    match std::fs::read_to_string(&expected_path) {
        Err(_) => (
            Some(normalized),
            CaseResult::skipped(
                &name,
                sail_file,
                "no checked-in .expected.lean for oracle self-check".to_string(),
            ),
        ),
        Ok(expected_raw) => {
            let expected_norm = crate::normalize::normalize(&expected_raw);
            match compare_outputs(&expected_norm, &normalized) {
                None => (
                    Some(normalized),
                    CaseResult {
                        name,
                        sail_file: sail_file.to_path_buf(),
                        status: CaseStatus::Pass,
                        detail: "oracle output matches checked-in expectation".to_string(),
                    },
                ),
                Some(diff) => (
                    Some(normalized),
                    CaseResult {
                        name,
                        sail_file: sail_file.to_path_buf(),
                        status: CaseStatus::OracleFailed,
                        detail: format!(
                            "oracle output drifted from checked-in {}:\n{}",
                            expected_path.display(),
                            diff
                        ),
                    },
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_kb_conversion() {
        let cfg = OracleConfig {
            memory_mb: 2,
            ..OracleConfig::default()
        };
        assert_eq!(cfg.memory_kb(), 2048);
    }

    #[test]
    fn command_carries_limits() {
        let cfg = OracleConfig {
            timeout_secs: 45,
            memory_mb: 1,
            ..OracleConfig::default()
        };
        let (prog, args) =
            build_oracle_command(&cfg, Path::new("/t/a.sail"), Path::new("/w/case-out"));
        assert_eq!(prog, "bash");
        let script = &args[1];
        assert!(script.contains("ulimit -v 1024"), "script: {script}");
        assert!(script.contains("timeout 45s"), "script: {script}");
        assert!(args.contains(&"--lean-output-dir".to_string()));
        assert!(args.contains(&"/w/case-out".to_string()));
        assert!(args.contains(&"/t/a.sail".to_string()));
    }

    #[test]
    fn classify_timeout() {
        let (s, d) = classify_exit(Some(124), "");
        assert_eq!(s, CaseStatus::Timeout);
        assert!(d.contains("124"));
    }

    #[test]
    fn classify_oom_by_code_and_trace() {
        let (s, _) = classify_exit(Some(137), "");
        assert_eq!(s, CaseStatus::OomSuspected);
        let (s, d) = classify_exit(None, "Killed\n");
        assert_eq!(s, CaseStatus::OomSuspected);
        assert!(d.contains("Killed"));
    }

    #[test]
    fn classify_generic_failure_keeps_tail() {
        let (s, d) = classify_exit(Some(1), "l1\nl2\nboom\n");
        assert_eq!(s, CaseStatus::OracleFailed);
        assert!(d.contains("exit 1"));
        assert!(d.contains("boom"));
    }

    #[test]
    fn last_lines_caps() {
        assert_eq!(last_lines("a\nb\nc\nd", 2), "c\nd");
        assert_eq!(last_lines("a", 5), "a");
        assert_eq!(last_lines("a\nb", 0), "");
    }

    #[test]
    fn compare_equal_is_none() {
        assert_eq!(compare_outputs("a\n", "a\n"), None);
    }

    #[test]
    fn compare_mismatch_is_some_diff() {
        let d = compare_outputs("a\n", "b\n").expect("must differ");
        assert!(d.contains("-a"));
        assert!(d.contains("+b"));
    }

    #[test]
    fn discover_finds_only_sail_sorted() {
        let base = std::env::temp_dir().join(format!(
            "sail-oracle-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("b.sail"), "b").unwrap();
        std::fs::write(base.join("a.sail"), "a").unwrap();
        std::fs::write(base.join("note.txt"), "x").unwrap();
        let got = discover_tests(&base).unwrap();
        assert_eq!(got, vec![base.join("a.sail"), base.join("b.sail")]);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn collect_joins_defs_and_top_upstream_style() {
        let base = std::env::temp_dir().join(format!(
            "sail-oracle-collect-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let out_dir = base.join("case-out");
        std::fs::create_dir_all(out_dir.join("out").join("Out")).unwrap();
        std::fs::write(out_dir.join("out").join("Out").join("Defs.lean"), "defs\n").unwrap();
        std::fs::write(out_dir.join("out").join("Out.lean"), "top\n").unwrap();
        // Lake scaffolding must be ignored, not break collection.
        std::fs::write(out_dir.join("out").join("lakefile.toml"), "x").unwrap();
        let got = collect_oracle_output(&out_dir).unwrap();
        assert_eq!(got, "defs\n\nXXXXXXXXX\n\ntop\n");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn collect_reports_missing_files() {
        let base = std::env::temp_dir().join(format!(
            "sail-oracle-missing-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(base.join("case-out")).unwrap();
        let err = collect_oracle_output(&base.join("case-out")).unwrap_err();
        assert!(err.contains("Defs.lean"), "err: {err}");
        assert!(err.contains("Out.lean"), "err: {err}");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn stage_copies_sail_and_sibling_json() {
        let base = std::env::temp_dir().join(format!(
            "sail-oracle-stage-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let src = base.join("src");
        let scratch = base.join("scratch");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(src.join("g.sail"), "sail").unwrap();
        std::fs::write(src.join("g.json"), "{}").unwrap();
        let bare = stage_input(&src.join("g.sail"), &scratch).unwrap();
        assert_eq!(bare, "g.sail");
        assert_eq!(
            std::fs::read_to_string(scratch.join("g.sail")).unwrap(),
            "sail"
        );
        assert_eq!(
            std::fs::read_to_string(scratch.join("g.json")).unwrap(),
            "{}"
        );
        // Without a sibling json, staging still succeeds.
        std::fs::write(src.join("plain.sail"), "p").unwrap();
        assert_eq!(
            stage_input(&src.join("plain.sail"), &scratch).unwrap(),
            "plain.sail"
        );
        assert!(!scratch.join("plain.json").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn oversize_file_skips_without_running() {
        let base = std::env::temp_dir().join(format!(
            "sail-oracle-big-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&base).unwrap();
        let f = base.join("huge.sail");
        std::fs::write(&f, vec![b'x'; 100]).unwrap();
        let cfg = OracleConfig {
            max_file_bytes: 10,
            work_dir: base.join("work"),
            ..OracleConfig::default()
        };
        let (out, res) = run_oracle_case(&cfg, &f);
        assert!(out.is_none());
        assert_eq!(res.status, CaseStatus::Skipped);
        assert!(res.detail.contains("nightly-only"));
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn missing_backend_errs_honestly() {
        let b = MissingBackend;
        let err = b
            .emit_lean(Path::new("/t/a.sail"), Path::new("/t/a.rust.lean"))
            .unwrap_err();
        assert!(err.contains("not implemented"));
    }

    #[test]
    fn status_failure_set() {
        assert!(!CaseStatus::Pass.is_failure());
        assert!(!CaseStatus::Skipped.is_failure());
        assert!(CaseStatus::Mismatch.is_failure());
        assert!(CaseStatus::Timeout.is_failure());
        assert!(CaseStatus::OomSuspected.is_failure());
        assert!(CaseStatus::OracleFailed.is_failure());
        assert!(CaseStatus::HarnessError.is_failure());
    }
}
