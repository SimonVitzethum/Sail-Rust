#![forbid(unsafe_code)]
//! Output normalization: strip volatile bits so diffs show real changes only.
//!
//! Rules (deliberately conservative):
//! - `\r\n` / `\r` line endings become `\n`.
//! - Trailing whitespace per line is removed.
//! - Runs of blank lines collapse to a single blank line.
//! - Lines matching the OCaml `sail` startup banner (`Sail <version> ...`)
//!   are dropped; they carry no test signal.
//! - Caller-provided absolute path prefixes (temp dirs, repo dirs) are
//!   redacted to `<PATH>` so outputs are machine-independent.

/// Returns true for lines that are always volatile and carry no test signal.
pub fn is_volatile_line(trimmed_end: &str) -> bool {
    let t = trimmed_end.trim_start();
    // OCaml sail prints e.g. "Sail 0.20.3" startup banners on stderr in some
    // configurations; never part of `--lean` file output we compare.
    (t.starts_with("Sail ") && t.chars().nth(5).is_some_and(|c| c.is_ascii_digit()))
        || t.starts_with("Warning: emanating from generated code")
}

/// Normalize `text`, redacting each prefix in `redactions` to `<PATH>`.
pub fn normalize_with(text: &str, redactions: &[&str]) -> String {
    // Sort redactions longest-first so nested prefixes match greedily.
    let mut reds: Vec<&str> = redactions.to_vec();
    reds.sort_by_key(|r| core::cmp::Reverse(r.len()));

    let unix = text.replace("\r\n", "\n").replace('\r', "\n");
    // "a\n".split('\n') yields ["a", ""]; drop that artifact so a trailing
    // newline does not become a phantom blank line.
    let mut lines: Vec<&str> = unix.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    let mut out = String::with_capacity(unix.len());
    let mut prev_blank = false;
    for line in lines {
        let stripped = line.trim_end();
        if is_volatile_line(stripped) {
            continue;
        }
        let mut owned: Option<String> = None;
        for red in &reds {
            if red.is_empty() {
                continue;
            }
            let hay = owned.as_deref().unwrap_or(stripped);
            if hay.contains(red) {
                owned = Some(hay.replace(red, "<PATH>"));
            }
        }
        let final_line = owned.as_deref().unwrap_or(stripped);
        if final_line.trim().is_empty() {
            if prev_blank {
                continue;
            }
            prev_blank = true;
            out.push('\n');
        } else {
            prev_blank = false;
            out.push_str(final_line);
            out.push('\n');
        }
    }
    out
}

/// Normalize `text` without path redactions.
pub fn normalize(text: &str) -> String {
    normalize_with(text, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crlf_and_trailing_whitespace() {
        let got = normalize("a  \r\nb\t\r\n");
        assert_eq!(got, "a\nb\n");
    }

    #[test]
    fn blank_runs_collapse_but_single_kept() {
        let got = normalize("a\n\n\n\nb\n");
        assert_eq!(got, "a\n\nb\n");
    }

    #[test]
    fn leading_blanks_collapse_to_one() {
        let got = normalize("\n\n\na\n");
        assert_eq!(got, "\na\n");
    }

    #[test]
    fn sail_banner_dropped() {
        let got = normalize("Sail 0.20.3\nreal output\n");
        assert_eq!(got, "real output\n");
    }

    #[test]
    fn sail_prefix_without_version_kept() {
        // Must not drop Lean code that happens to start with "Sail ".
        let got = normalize("Sail library import\n");
        assert_eq!(got, "Sail library import\n");
    }

    #[test]
    fn redactions_replace_paths() {
        let got = normalize_with(
            "error at /tmp/xyz-1/out.lean:3\nsee /tmp/xyz-1 for details\n",
            &["/tmp/xyz-1"],
        );
        assert_eq!(got, "error at <PATH>/out.lean:3\nsee <PATH> for details\n");
    }

    #[test]
    fn longest_redaction_wins() {
        let got = normalize_with("p=/a/b/c", &["/a", "/a/b"]);
        assert_eq!(got, "p=<PATH>/c\n");
    }

    #[test]
    fn empty_input_stays_empty() {
        assert_eq!(normalize(""), "");
    }

    #[test]
    fn division_slashes_untouched() {
        let got = normalize("abbrev x := a / b\n");
        assert_eq!(got, "abbrev x := a / b\n");
    }
}
