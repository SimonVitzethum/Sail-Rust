#![forbid(unsafe_code)]
//! Markdown report builder: summary counts, failing list (`file + reason`),
//! skips, and per-case diff excerpts. Pure function over [`CaseResult`]s,
//! so CI can archive the output as an artifact.

use crate::oracle::{CaseResult, CaseStatus};

/// Context needed to interpret a report (versions, limits, backend).
pub struct ReportMeta {
    pub oracle_binary: String,
    pub oracle_version: String,
    pub test_dir: String,
    pub timeout_secs: u64,
    pub memory_mb: u64,
    pub rust_backend: String,
}

/// First line of `detail` (or the whole string when single-line).
fn first_line(detail: &str) -> &str {
    match detail.find('\n') {
        Some(i) => detail[..i].trim_end(),
        None => detail.trim(),
    }
}

/// Cap `detail` to `max_lines` lines for the failing table.
fn excerpt(detail: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = detail.lines().take(max_lines).collect();
    let mut s = lines.join("\n");
    if detail.lines().count() > max_lines {
        s.push_str("\n… (truncated, see Diffs section)");
    }
    s
}

/// Build the full Markdown report.
pub fn build_markdown(results: &[CaseResult], meta: &ReportMeta) -> String {
    let total = results.len();
    let passed = results
        .iter()
        .filter(|r| r.status == CaseStatus::Pass)
        .count();
    let skipped = results
        .iter()
        .filter(|r| r.status == CaseStatus::Skipped)
        .count();
    let failing: Vec<&CaseResult> = results.iter().filter(|r| r.is_failure()).collect();
    let verdict = if failing.is_empty() { "PASS" } else { "FAIL" };

    let mut out = String::new();
    out.push_str("# Sail Oracle Diff Report\n\n");
    out.push_str(&format!(
        "Oracle: `{}` ({})\n\nTest dir: `{}`\n\nLimits: timeout {timeout}s, RAM {mem} MiB per run\n\nRust backend: {backend}\n\n",
        meta.oracle_binary,
        meta.oracle_version,
        meta.test_dir,
        timeout = meta.timeout_secs,
        mem = meta.memory_mb,
        backend = meta.rust_backend,
    ));
    out.push_str("## Summary\n\n");
    out.push_str(&format!(
        "- Verdict: **{verdict}**\n- Total: {total}, passed: {passed}, failing: {n}, skipped: {skipped}\n\n",
        n = failing.len(),
    ));

    out.push_str("## Failing tests\n\n");
    if failing.is_empty() {
        out.push_str("None — all executed cases match the oracle.\n\n");
    } else {
        out.push_str("| Test file | Reason | Detail |\n");
        out.push_str("| --- | --- | --- |\n");
        for r in &failing {
            let file = r.sail_file.to_string_lossy().replace('|', "\\|");
            let reason = r.status.reason().replace('|', "\\|");
            let detail = excerpt(first_line(&r.detail), 1).replace('|', "\\|");
            out.push_str(&format!("| `{file}` | {reason} | {detail} |\n"));
        }
        out.push('\n');
    }

    let skipped_cases: Vec<&CaseResult> = results
        .iter()
        .filter(|r| r.status == CaseStatus::Skipped)
        .collect();
    out.push_str("## Skipped\n\n");
    if skipped_cases.is_empty() {
        out.push_str("None.\n\n");
    } else {
        for r in &skipped_cases {
            out.push_str(&format!(
                "- `{}` — {}\n",
                r.sail_file.to_string_lossy(),
                first_line(&r.detail)
            ));
        }
        out.push('\n');
    }

    let with_diffs: Vec<&CaseResult> = failing
        .iter()
        .filter(|r| r.status == CaseStatus::Mismatch || r.status == CaseStatus::OracleFailed)
        .copied()
        .collect();
    out.push_str("## Diffs\n\n");
    if with_diffs.is_empty() {
        out.push_str("No diffs to show.\n\n");
    } else {
        for r in &with_diffs {
            out.push_str(&format!(
                "<details><summary>{} ({})</summary>\n\n```diff\n{}\n```\n\n</details>\n\n",
                r.name,
                r.status.reason(),
                excerpt(&r.detail, 200),
            ));
        }
    }

    let passed_cases: Vec<&CaseResult> = results
        .iter()
        .filter(|r| r.status == CaseStatus::Pass)
        .collect();
    out.push_str("## Passed\n\n");
    if passed_cases.is_empty() {
        out.push_str("None.\n");
    } else {
        let names: Vec<String> = passed_cases
            .iter()
            .map(|r| format!("`{}`", r.name))
            .collect();
        out.push_str(&format!("{} passed: {}\n", names.len(), names.join(", ")));
    }
    if total == 0 {
        out.push_str("\n> Note: no test cases discovered — check `--test-dir`.\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn meta() -> ReportMeta {
        ReportMeta {
            oracle_binary: "/bin/sail".to_string(),
            oracle_version: "Sail 0.20.3".to_string(),
            test_dir: "/t".to_string(),
            timeout_secs: 120,
            memory_mb: 2048,
            rust_backend: "missing".to_string(),
        }
    }

    fn case(name: &str, status: CaseStatus, detail: &str) -> CaseResult {
        CaseResult {
            name: name.to_string(),
            sail_file: PathBuf::from(format!("/t/{name}.sail")),
            status,
            detail: detail.to_string(),
        }
    }

    #[test]
    fn failing_list_has_file_and_reason() {
        let results = vec![
            case("good", CaseStatus::Pass, "ok"),
            case("bad", CaseStatus::Mismatch, "-a\n+b"),
            case("slow", CaseStatus::Timeout, "killed by timeout(1)"),
        ];
        let md = build_markdown(&results, &meta());
        assert!(md.contains("Verdict: **FAIL**"));
        assert!(md.contains("/t/bad.sail"));
        assert!(md.contains("output mismatch vs oracle"));
        assert!(md.contains("/t/slow.sail"));
        assert!(md.contains("oracle timeout"));
        // passing case not in failing table
        assert!(!md.contains("| `/t/good.sail` |"));
        // diff excerpt present
        assert!(md.contains("-a"));
        // summary counts
        assert!(md.contains("Total: 3, passed: 1, failing: 2, skipped: 0"));
    }

    #[test]
    fn all_green_report() {
        let results = vec![case("good", CaseStatus::Pass, "ok")];
        let md = build_markdown(&results, &meta());
        assert!(md.contains("Verdict: **PASS**"));
        assert!(md.contains("None — all executed cases match the oracle."));
        assert!(md.contains("`good`"));
    }

    #[test]
    fn empty_results_note() {
        let md = build_markdown(&[], &meta());
        assert!(md.contains("no test cases discovered"));
        assert!(md.contains("Total: 0"));
    }

    #[test]
    fn skips_listed_with_reason() {
        let results = vec![case(
            "huge",
            CaseStatus::Skipped,
            "9000 bytes > 8192 byte suite cap (nightly-only, e.g. ARM)",
        )];
        let md = build_markdown(&results, &meta());
        assert!(md.contains("## Skipped"));
        assert!(md.contains("huge"));
        assert!(md.contains("nightly-only"));
        assert!(md.contains("Verdict: **PASS**"));
    }

    #[test]
    fn pipes_escaped_in_table() {
        let results = vec![case("x", CaseStatus::OracleFailed, "a|b")];
        let md = build_markdown(&results, &meta());
        assert!(md.contains("a\\|b"));
    }
}
