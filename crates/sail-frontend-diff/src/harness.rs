#![forbid(unsafe_code)]
//! Harness wiring: oracle dump invocation, both-side elaboration, verdicts.
//!
//! Oracle side runs `sail --just-check <dump-flag> <file>` under the same
//! limits convention as `sail-oracle` (`bash -c 'ulimit -v <kb>; exec
//! timeout <s>s …'`, `cwd` = staged scratch with the bare file name, so
//! generated paths stay identical to upstream runs). Rust side parses with
//! `sail_parse::parse_file` and normalizes via [`crate::norm_tree`].
//!
//! E3 note: this module is shipped as code; the Rig run happens later
//! (see `ARCHITECTURE.md`). Nothing here spawns processes at
//! import time — only [`run_frontend_case`] does, and only the Rig calls it.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Dump flag for the untyped (parse) AST.
pub const DUMP_INITIAL: &str = "--ddump-initial-ast";
/// Dump flag for the typechecked AST (type-judgment samples).
pub const DUMP_TC: &str = "--ddump-tc-ast";

#[derive(Debug, Clone)]
pub struct FrontendConfig {
    pub sail_binary: PathBuf,
    pub test_dir: PathBuf,
    pub timeout_secs: u64,
    pub memory_mb: u64,
    pub max_file_bytes: u64,
    pub work_dir: PathBuf,
    /// Type-judgment samples per file (deterministic first-N).
    pub ty_samples_per_file: usize,
}

impl Default for FrontendConfig {
    fn default() -> Self {
        Self {
            sail_binary: PathBuf::from("/home/simon/.opam/sail/bin/sail"),
            test_dir: PathBuf::from("/home/simon/Dokumente/Sail/sail/test/lean"),
            timeout_secs: 120,
            memory_mb: 2048,
            max_file_bytes: crate::catalog::MAX_FILE_BYTES,
            work_dir: std::env::temp_dir().join("sail-frontend-diff"),
            ty_samples_per_file: 8,
        }
    }
}

impl FrontendConfig {
    /// Virtual-memory cap in KiB for `ulimit -v`.
    pub fn memory_kb(&self) -> u64 {
        self.memory_mb.saturating_mul(1024)
    }
}

/// Outcome of one frontend differential case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrontendStatus {
    /// Parse trees match and all type samples match.
    ParseMatch,
    /// Normalized parse trees differ; `detail` holds summary + seq diff.
    ParseMismatch,
    /// Parse trees match but at least one type sample differs; `detail`
    /// holds the per-sample verdicts.
    TySampleMismatch,
    /// The oracle itself failed; `detail` holds exit + stderr tail.
    OracleFailed,
    /// Wall-clock limit hit (`timeout(1)` exit 124).
    Timeout,
    /// SIGKILL / OOM-killer traces.
    OomSuspected,
    /// Not run; `detail` is the reason.
    Skipped,
    /// The harness (not the oracle) broke — including dump-sifter
    /// anomalies (empty sift on non-empty input); `detail` is the message.
    HarnessError,
}

impl FrontendStatus {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::ParseMatch => "parse trees match",
            Self::ParseMismatch => "parse-tree mismatch vs oracle",
            Self::TySampleMismatch => "type-sample mismatch vs oracle",
            Self::OracleFailed => "oracle run failed",
            Self::Timeout => "oracle timeout",
            Self::OomSuspected => "suspected oracle OOM",
            Self::Skipped => "skipped",
            Self::HarnessError => "harness error",
        }
    }

    pub fn is_failure(&self) -> bool {
        matches!(
            self,
            Self::ParseMismatch
                | Self::TySampleMismatch
                | Self::OracleFailed
                | Self::Timeout
                | Self::OomSuspected
                | Self::HarnessError
        )
    }
}

#[derive(Debug, Clone)]
pub struct FrontendResult {
    /// Test name = `.sail` file stem.
    pub name: String,
    pub sail_file: PathBuf,
    pub status: FrontendStatus,
    /// Summary / diff / reason depending on `status`.
    pub detail: String,
}

impl FrontendResult {
    pub fn skipped(name: &str, sail_file: &Path, reason: impl Into<String>) -> Self {
        Self {
            name: name.to_string(),
            sail_file: sail_file.to_path_buf(),
            status: FrontendStatus::Skipped,
            detail: reason.into(),
        }
    }

    pub fn is_failure(&self) -> bool {
        self.status.is_failure()
    }
}

/// Build the `(program, args)` for one oracle dump run under limits.
///
/// `sail_file` is the (bare, staged) input name, `dump_flag` is
/// [`DUMP_INITIAL`] or [`DUMP_TC`]. Pure constructor, unit-tested.
pub fn build_dump_command(
    cfg: &FrontendConfig,
    sail_file: &Path,
    dump_flag: &str,
) -> (String, Vec<String>) {
    let script = format!(
        "ulimit -v {}; exec timeout {}s \"$@\"",
        cfg.memory_kb(),
        cfg.timeout_secs
    );
    let mut args = vec!["-c".to_string(), script, "sail-frontend-diff".to_string()];
    args.push(cfg.sail_binary.to_string_lossy().into_owned());
    args.push("--just-check".to_string());
    args.push(dump_flag.to_string());
    args.push(sail_file.to_string_lossy().into_owned());
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

/// Run one oracle dump; returns stdout on success or a `(status, detail)`
/// failure. `redact` prefixes are stripped from stdout via normalization.
fn run_dump(
    cfg: &FrontendConfig,
    scratch: &Path,
    staged: &str,
    dump_flag: &str,
) -> Result<String, (FrontendStatus, String)> {
    let (prog, args) = build_dump_command(cfg, Path::new(staged), dump_flag);
    let output = std::process::Command::new(&prog)
        .args(&args)
        .current_dir(scratch)
        .output()
        .map_err(|e| {
            (
                FrontendStatus::HarnessError,
                format!("cannot spawn oracle ({dump_flag}): {e}"),
            )
        })?;
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let (oc, detail) = sail_oracle::oracle::classify_exit(output.status.code(), &stderr);
    match oc {
        sail_oracle::oracle::CaseStatus::Pass => {}
        sail_oracle::oracle::CaseStatus::Timeout => {
            return Err((FrontendStatus::Timeout, detail));
        }
        sail_oracle::oracle::CaseStatus::OomSuspected => {
            return Err((FrontendStatus::OomSuspected, detail));
        }
        _ => {
            return Err((FrontendStatus::OracleFailed, detail));
        }
    }
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let red = scratch.to_string_lossy().into_owned();
    Ok(sail_oracle::normalize::normalize_with(&stdout, &[red.as_str()]))
}

/// Run one frontend differential case (parse trees + type samples).
///
/// Only the Rig calls this (E3: not executed in this lane). Steps:
/// size cap -> stage -> initial-AST dump -> Rust parse -> positional compare
/// -> (unless catalog says [`crate::catalog::Focus::ParseOnly`]) tc-AST dump
/// -> sample compare. Empty oracle sifts on non-empty dumps are
/// `HarnessError`, never passes.
pub fn run_frontend_case(cfg: &FrontendConfig, sail_file: &Path) -> FrontendResult {
    let name = sail_file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| sail_file.to_string_lossy().into_owned());
    let fail = |status: FrontendStatus, detail: String| FrontendResult {
        name: name.clone(),
        sail_file: sail_file.to_path_buf(),
        status,
        detail,
    };

    let size = match std::fs::metadata(sail_file) {
        Ok(m) => m.len(),
        Err(e) => return fail(FrontendStatus::HarnessError, format!("cannot stat input: {e}")),
    };
    if size > cfg.max_file_bytes {
        return FrontendResult::skipped(
            &name,
            sail_file,
            format!(
                "{size} bytes > {} byte suite cap (nightly-only)",
                cfg.max_file_bytes
            ),
        );
    }

    let scratch = unique_dir(&cfg.work_dir, &name);
    if let Err(e) = std::fs::create_dir_all(&scratch) {
        return fail(
            FrontendStatus::HarnessError,
            format!("cannot create scratch {}: {e}", scratch.display()),
        );
    }
    let staged = match sail_oracle::oracle::stage_input(sail_file, &scratch) {
        Ok(f) => f,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&scratch);
            return fail(FrontendStatus::HarnessError, format!("cannot stage input: {e}"));
        }
    };

    let cleanup = |scratch: &Path| {
        let _ = std::fs::remove_dir_all(scratch);
    };

    // --- Oracle parse dump -------------------------------------------------
    let oracle_text = match run_dump(cfg, &scratch, &staged, DUMP_INITIAL) {
        Ok(t) => t,
        Err((status, detail)) => {
            cleanup(&scratch);
            return fail(status, detail);
        }
    };
    let oracle_defs = crate::norm_tree::sift_oracle_dump(&oracle_text);
    if oracle_defs.is_empty() && !oracle_text.trim().is_empty() {
        cleanup(&scratch);
        return fail(
            FrontendStatus::HarnessError,
            "initial-AST sifter found no DEF_ entries in non-empty oracle dump \
             (dump format drift? verify --ddump-initial-ast shape at Rig run)"
                .to_string(),
        );
    }

    // --- Rust parse --------------------------------------------------------
    let src = match std::fs::read_to_string(&scratch.join(&staged)) {
        Ok(s) => s,
        Err(e) => {
            cleanup(&scratch);
            return fail(FrontendStatus::HarnessError, format!("cannot read staged input: {e}"));
        }
    };
    let parsed = match sail_parse::parse_file(&staged, &src) {
        Ok(p) => p,
        Err(e) => {
            cleanup(&scratch);
            return fail(
                FrontendStatus::ParseMismatch,
                format!("Rust parser rejected input that the oracle dumped: {e:?}"),
            );
        }
    };
    let (rust_defs, dropped) = crate::norm_tree::from_rust(&parsed);
    let expected: Vec<crate::norm_tree::NormDef> =
        oracle_defs.into_iter().map(crate::norm_tree::NormDef::from).collect();
    let report = crate::tree_compare::compare(&expected, &rust_defs);
    if !report.is_clean() {
        let mut detail = format!(
            "{} ({} shard markers dropped)\n{}",
            crate::tree_compare::summarize(&report),
            dropped,
            crate::tree_compare::render_seq_diff(&expected, &rust_defs),
        );
        // Cap the detail: full seq diffs of ARM-scale files are huge.
        const MAX_DETAIL: usize = 32_000;
        if detail.len() > MAX_DETAIL {
            detail.truncate(MAX_DETAIL);
            detail.push_str("\n… (detail truncated)");
        }
        cleanup(&scratch);
        return fail(FrontendStatus::ParseMismatch, detail);
    }

    // --- Type-judgment sampling --------------------------------------------
    let catalog_focus = crate::catalog::find(&name).map(|c| c.focus);
    if catalog_focus == Some(crate::catalog::Focus::ParseOnly) {
        cleanup(&scratch);
        return fail(
            FrontendStatus::Skipped,
            format!(
                "{}; type sampling skipped (catalog: ParseOnly)",
                crate::tree_compare::summarize(&report),
            ),
        );
    }
    let oracle_tc = match run_dump(cfg, &scratch, &staged, DUMP_TC) {
        Ok(t) => t,
        Err((status, detail)) => {
            cleanup(&scratch);
            // A tc-dump failure after a good parse dump is oracle-side news,
            // not a parse verdict change.
            return fail(status, format!("parse trees match; tc-dump failed: {detail}"));
        }
    };
    let oracle_samples = crate::ty_sample::sift_tc_dump(&oracle_tc);
    let requests = crate::ty_sample::sample_requests(&rust_defs, cfg.ty_samples_per_file);
    // Rust scheme rendering for sampled defs: at the skeleton layer the only
    // honest rendering is the normalized header text (full `sail-ty`
    // elaboration per sampled def is Lane 05+ follow-up work; see plan).
    let rust_samples: Vec<(String, String)> = requests
        .iter()
        .filter_map(|r| {
            rust_defs
                .iter()
                .find(|d| d.name.as_deref() == Some(r.def_name.as_str()))
                .map(|d| {
                    (
                        r.def_name.clone(),
                        crate::ty_sample::normalize_scheme(&d.head_norm),
                    )
                })
        })
        .collect();
    let ty_verdicts = crate::ty_sample::compare_samples(&oracle_samples, &rust_samples);
    let mismatches: Vec<&crate::ty_sample::TyVerdict> =
        ty_verdicts.iter().filter(|v| !v.is_match()).collect();
    cleanup(&scratch);
    if mismatches.is_empty() {
        FrontendResult {
            name,
            sail_file: sail_file.to_path_buf(),
            status: FrontendStatus::ParseMatch,
            detail: format!(
                "{}; {} type samples match ({} requested)",
                crate::tree_compare::summarize(&report),
                ty_verdicts.len(),
                requests.len(),
            ),
        }
    } else {
        let mut detail = format!("{} type-sample mismatches:\n", mismatches.len());
        for v in mismatches.iter().take(16) {
            detail.push_str(&format!("- {}: {}\n", v.def_name, v.detail));
        }
        fail(FrontendStatus::TySampleMismatch, detail)
    }
}
