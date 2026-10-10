#![forbid(unsafe_code)]
//! Minimal unified diff over lines (std-only, no external crates).
//!
//! Strategy: trim common prefix/suffix, then run an LCS dynamic program on
//! the differing middle. If the middle is too large (`> MAX_MIDDLE_CELLS`
//! DP cells), fall back to emitting the whole block as changed instead of
//! allocating unbounded memory — the harness itself must stay low-RAM.

/// Maximum DP cells for the LCS on the differing middle.
pub const MAX_MIDDLE_CELLS: usize = 4_000_000;
/// Maximum number of diff lines emitted before truncation kicks in.
pub const MAX_DIFF_LINES: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Same,
    Del,
    Add,
}

/// Compute the op script for the differing middle via LCS DP.
///
/// `a`/`b` are the middle slices. Returns `None` when the DP would exceed
/// `MAX_MIDDLE_CELLS`.
fn lcs_ops(a: &[&str], b: &[&str]) -> Option<Vec<Op>> {
    let n = a.len();
    let m = b.len();
    if n.saturating_mul(m) > MAX_MIDDLE_CELLS {
        return None;
    }
    // lengths[i][j] = LCS length of a[i..] and b[j..]; rolling rows suffice,
    // but backtracking needs the full table. (n+1)*(m+1) u32 entries.
    let cols = m + 1;
    let mut len = vec![0u32; (n + 1) * cols];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            len[i * cols + j] = if a[i] == b[j] {
                len[(i + 1) * cols + (j + 1)] + 1
            } else {
                len[(i + 1) * cols + j].max(len[i * cols + (j + 1)])
            };
        }
    }
    let mut ops = Vec::with_capacity(n + m);
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            ops.push(Op::Same);
            i += 1;
            j += 1;
        } else if len[(i + 1) * cols + j] >= len[i * cols + (j + 1)] {
            ops.push(Op::Del);
            i += 1;
        } else {
            ops.push(Op::Add);
            j += 1;
        }
    }
    while i < n {
        ops.push(Op::Del);
        i += 1;
    }
    while j < m {
        ops.push(Op::Add);
        j += 1;
    }
    Some(ops)
}

/// One hunk of a unified diff (1-based line numbers into each side).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub expected_start: usize,
    pub actual_start: usize,
    pub expected_count: usize,
    pub actual_count: usize,
    /// Lines prefixed later with ' ', '-', '+'.
    pub ops: Vec<(Op, String)>,
}

/// Split two normalized texts into hunks with `context` lines of context.
/// Returns an empty vec when the texts are identical.
pub fn hunks(expected: &str, actual: &str, context: usize) -> Vec<Hunk> {
    let a: Vec<&str> = expected.split('\n').collect();
    let b: Vec<&str> = actual.split('\n').collect();
    // Drop the artifact of trailing-newline splitting: "x\n" -> ["x", ""].
    let mut a = a.as_slice();
    let mut b = b.as_slice();
    if a.last() == Some(&"") {
        a = &a[..a.len() - 1];
    }
    if b.last() == Some(&"") {
        b = &b[..b.len() - 1];
    }

    let mut pre = 0usize;
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0usize;
    while suf < a.len() - pre && suf < b.len() - pre && a[a.len() - 1 - suf] == b[b.len() - 1 - suf]
    {
        suf += 1;
    }
    if pre == a.len() && pre == b.len() {
        return Vec::new(); // identical
    }
    let mid_a = &a[pre..a.len() - suf];
    let mid_b = &b[pre..b.len() - suf];

    // Walk the LCS script, tracking positions inside the middles.
    // Fallback when the middle is too large: whole block changed, no LCS.
    let script: Vec<Op> = match lcs_ops(mid_a, mid_b) {
        Some(s) => s,
        // Fallback: whole middle changed, no LCS.
        None => mid_a
            .iter()
            .map(|_| Op::Del)
            .chain(mid_b.iter().map(|_| Op::Add))
            .collect(),
    };

    // Walk `script`, tracking positions inside the middles.
    let mut tagged: Vec<(Op, String)> = Vec::with_capacity(script.len());
    let (mut ia, mut ib) = (0usize, 0usize);
    for op in script {
        match op {
            Op::Same => {
                tagged.push((Op::Same, mid_a[ia].to_string()));
                ia += 1;
                ib += 1;
            }
            Op::Del => {
                tagged.push((Op::Del, mid_a[ia].to_string()));
                ia += 1;
            }
            Op::Add => {
                tagged.push((Op::Add, mid_b[ib].to_string()));
                ib += 1;
            }
        }
    }

    // Assemble full op stream with absolute line numbers, then cut hunks.
    // Full stream: pre Same-lines, middle tagged, suf Same-lines.
    let mut full: Vec<(Op, String)> = Vec::with_capacity(pre + tagged.len() + suf);
    for l in &a[..pre] {
        full.push((Op::Same, (*l).to_string()));
    }
    full.extend(tagged);
    for l in &a[a.len() - suf..] {
        full.push((Op::Same, (*l).to_string()));
    }

    // Indices in `full` that are changes; group with `context` padding.
    let change_idx: Vec<usize> = full
        .iter()
        .enumerate()
        .filter(|(_, (op, _))| *op != Op::Same)
        .map(|(i, _)| i)
        .collect();
    if change_idx.is_empty() {
        return Vec::new();
    }
    let mut groups: Vec<(usize, usize)> = Vec::new();
    let mut gs = change_idx[0].saturating_sub(context);
    let mut ge = (change_idx[0] + context).min(full.len().saturating_sub(1));
    for &ci in &change_idx[1..] {
        let s = ci.saturating_sub(context);
        let e = (ci + context).min(full.len().saturating_sub(1));
        if s <= ge + 1 {
            ge = ge.max(e);
        } else {
            groups.push((gs, ge));
            gs = s;
            ge = e;
        }
    }
    groups.push((gs, ge));

    // Convert groups to hunks with line numbers.
    let mut hunks_out = Vec::new();
    // Line counters: walk `full` once.
    // ea/be = current 1-based line number in expected/actual.
    let mut ea = 1usize;
    let mut ba = 1usize;
    // Map full-index -> (ea, ba) at that index.
    let mut pos: Vec<(usize, usize)> = Vec::with_capacity(full.len());
    for (op, _) in &full {
        pos.push((ea, ba));
        match op {
            Op::Same => {
                ea += 1;
                ba += 1;
            }
            Op::Del => ea += 1,
            Op::Add => ba += 1,
        }
    }
    for (s, e) in groups {
        let slice = &full[s..=e];
        let (es, bs) = pos[s];
        let ec = slice.iter().filter(|(op, _)| *op != Op::Add).count();
        let ac = slice.iter().filter(|(op, _)| *op != Op::Del).count();
        hunks_out.push(Hunk {
            expected_start: es,
            actual_start: bs,
            expected_count: ec,
            actual_count: ac,
            ops: slice.to_vec(),
        });
    }
    hunks_out
}

/// Render hunks as a unified diff with `--- expected` / `+++ actual`.
/// Returns an empty string when inputs are identical.
pub fn unified_diff(expected: &str, actual: &str, context: usize) -> String {
    unified_diff_capped(expected, actual, context, MAX_DIFF_LINES)
}

fn unified_diff_capped(expected: &str, actual: &str, context: usize, max_lines: usize) -> String {
    let hs = hunks(expected, actual, context);
    if hs.is_empty() {
        return String::new();
    }
    let mut out = String::from("--- expected (oracle)\n+++ actual (rust)\n");
    let mut lines = 2usize;
    for h in &hs {
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            h.expected_start, h.expected_count, h.actual_start, h.actual_count
        ));
        lines += 1;
        for (op, text) in &h.ops {
            if lines >= max_lines {
                out.push_str("... (diff truncated)\n");
                return out;
            }
            let sigil = match op {
                Op::Same => ' ',
                Op::Del => '-',
                Op::Add => '+',
            };
            out.push(sigil);
            out.push_str(text);
            out.push('\n');
            lines += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_is_empty() {
        assert_eq!(unified_diff("a\nb\n", "a\nb\n", 3), "");
        assert_eq!(unified_diff("", "", 3), "");
    }

    #[test]
    fn trailing_newline_difference_ignored() {
        assert_eq!(unified_diff("a\n", "a", 3), "");
    }

    #[test]
    fn simple_change_has_markers() {
        let d = unified_diff("a\nb\nc\n", "a\nB\nc\n", 1);
        assert!(d.contains("--- expected (oracle)"));
        assert!(d.contains("+++ actual (rust)"));
        assert!(d.contains("-b"));
        assert!(d.contains("+B"));
        assert!(d.contains("@@ "));
        // context lines kept
        assert!(d.contains(" a"));
        assert!(d.contains(" c"));
    }

    #[test]
    fn pure_addition() {
        let d = unified_diff("a\n", "a\nb\n", 0);
        assert!(d.contains("+b"));
        assert!(!d.contains("-b"));
    }

    #[test]
    fn hunk_headers_are_one_based() {
        let d = unified_diff("x\ny\n", "x\nz\n", 0);
        assert!(d.contains("@@ -2,1 +2,1 @@"));
    }

    #[test]
    fn large_middle_falls_back_without_hanging() {
        // 3000 x 3000 cells = 9M > MAX_MIDDLE_CELLS -> fallback path.
        let a = (0..3000)
            .map(|i| format!("a{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let b = (0..3000)
            .map(|i| format!("b{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let d = unified_diff(&a, &b, 0);
        assert!(d.contains("-a0"));
        assert!(d.contains("+b0"));
    }

    #[test]
    fn truncation_kicks_in() {
        let a = (0..500)
            .map(|i| format!("a{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let b = (0..500)
            .map(|i| format!("b{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let d = unified_diff_capped(&a, &b, 0, 10);
        assert!(d.contains("... (diff truncated)"));
    }
}
