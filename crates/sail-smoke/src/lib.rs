#![forbid(unsafe_code)]
//! Smoke parity fixtures Lean-vs-C (lane 39).
//!
//! Compares expected behaviour of small Sail models between the
//! Rust-Lean output and the OCaml-C output. This crate is NOT a C-backend
//! rewrite: it holds a fixture table of 10 decode/execute cases over
//! vector/int examples from `sail/test/lean/*.sail`, documents the expected
//! behaviour from each `.expected.lean` plus the OCaml-C semantics, and
//! provides comparison logic over log snippets plus a runner sketch that
//! will later invoke both binaries.

use core::fmt;

/// One smoke case: a tiny decode/execute scenario.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SmokeCase {
    /// Stable id, e.g. `"bitvec-eq"`.
    pub id: &'static str,
    /// Source Sail file under `sail/test/lean/`, e.g. `"bitvec_operation.sail"`.
    pub sail_source: &'static str,
    /// Sail function under test, e.g. `"bitvector_eq"`.
    pub sail_function: &'static str,
    /// Short description of the decode/execute scenario.
    pub description: &'static str,
    /// Expected behaviour per `.expected.lean` (Lean side).
    pub lean_expect: &'static str,
    /// Expected behaviour per OCaml-C semantics (C side).
    pub c_expect: &'static str,
    /// Representative Lean-binary log snippet for this case.
    pub lean_log: &'static str,
    /// Representative C-binary log snippet for this case.
    pub c_log: &'static str,
}

/// The 10 smoke fixtures.
pub const SMOKE_CASES: [SmokeCase; 10] = [
    SmokeCase {
        id: "bitvec-eq",
        sail_source: "bitvec_operation.sail",
        sail_function: "bitvector_eq(x, y)",
        description: "Execute 16-bit equality: x == y over bits(16).",
        lean_expect: "Lean: `(BitVec 16) -> (BitVec 16) -> Bool`; `==` on BitVec, from bitvec_operation.expected.lean `bits k_n := BitVec k_n`.",
        c_expect: "C (OCaml backend): `eq_bits(x, y)` over `uint16_t` limbs; same boolean result, no width truncation.",
        lean_log: "bitvector_eq(0x00FF, 0x00FF) = true",
        c_log: "bitvector_eq(0x00FF, 0x00FF) = true",
    },
    SmokeCase {
        id: "bitvec-add",
        sail_source: "bitvec_operation.sail",
        sail_function: "bitvector_add(x, y)",
        description: "Execute 16-bit wraparound add: x + y over bits(16).",
        lean_expect: "Lean: `BitVec.add` truncated to 16 bits; `0xFFFF + 1 = 0x0000`.",
        c_expect: "C: `add_bits` with masking to 16 bits; identical wraparound, same hex rendering.",
        lean_log: "bitvector_add(0xFFFF, 0x0001) = 0x0000",
        c_log: "bitvector_add(0xFFFF, 0x0001) = 0x0000",
    },
    SmokeCase {
        id: "bitvec-append",
        sail_source: "bitvec_operation.sail",
        sail_function: "bitvector_append(x, y)",
        description: "Execute append of two bits(16) into bits(32).",
        lean_expect: "Lean: `x +++ y : BitVec 32` (E_vector_append), MSB-first order.",
        c_expect: "C: `append_bits(x, y)` concatenating limbs MSB-first; same 32-bit hex string.",
        lean_log: "bitvector_append(0xABCD, 0x1234) = 0xABCD1234",
        c_log: "bitvector_append(0xABCD, 0x1234) = 0xABCD1234",
    },
    SmokeCase {
        id: "bitvec-index",
        sail_source: "bitvec_operation.sail",
        sail_function: "bitvector_access'(x, i)",
        description: "Execute bit index: x[i] for range(0,15) over bits(16).",
        lean_expect: "Lean: `BitVec.getMsb`/`getLsb` access; `0b1000...[0]` decodes MSB-first per Order dec.",
        c_expect: "C: `access_bits` with bounds check on index; same bit value, out-of-range is a runtime error on both sides.",
        lean_log: "bitvector_access'(0x8000, 0) = 1",
        c_log: "bitvector_access'(0x8000, 0) = 1",
    },
    SmokeCase {
        id: "bitfield-cr",
        sail_source: "bitfield.sail",
        sail_function: "register R : cr_type",
        description: "Decode bitfield cr_type over bits(8): fields CR0(7..4), LT(7), GT(6), CR1(3..2), CR3(1..0).",
        lean_expect: "Lean: `abbrev cr_type := BitVec 8`; field access via `extractLsb`; bitfield.expected.lean maps Register R to BitVec 8.",
        c_expect: "C: struct-of-masks with `get_field`/`set_field` shifts; same extracted subranges, same write-back value.",
        lean_log: "R.CR0 = 0xA where R = 0xA5",
        c_log: "R.CR0 = 0xA where R = 0xA5",
    },
    SmokeCase {
        id: "decode-clause-a",
        sail_source: "match_bv.sail",
        sail_function: "decode(32-bit pattern clause 1)",
        description: "Decode 32-bit word against first match_bv clause (`0b11@0b111@...` with Rm/Rn fields); Rm==Rn yields true.",
        lean_expect: "Lean: `P_vector_concat` match compiled to nested `BitVec` destructuring; guard `Rm == Rn` decides true/false.",
        c_expect: "C: `match_bits` switch over masked constants plus equality guard; first-match-wins, same clause selected.",
        lean_log: "decode(0b1111100...) clause=1 result=true",
        c_log: "decode(0b1111100...) clause=1 result=true",
    },
    SmokeCase {
        id: "decode-guard",
        sail_source: "match_bv.sail",
        sail_function: "write_CSR(12-bit guarded clause)",
        description: "Decode 12-bit CSR word with guard `unsigned(index) >= 3`; guarded clause only fires when guard holds.",
        lean_expect: "Lean: pattern guard becomes `if unsigned index >= 3 then ... else fallthrough` in the match elaboration.",
        c_expect: "C: `if` after mask check; guarded clause skipped when guard false, falls to next clause identically.",
        lean_log: "write_CSR(0xB03) guard=true result=true",
        c_log: "write_CSR(0xB03) guard=true result=true",
    },
    SmokeCase {
        id: "gpr-vector",
        sail_source: "register_vector.sail",
        sail_function: "rX(n) / wX(n, value)",
        description: "Execute register-vector access: GPRs[n] dereference with x31 hardwired to zero.",
        lean_expect: "Lean: `GPRs : Vector (RegisterRef ... 64) 31`; rX(31) returns `0x0`, wX(31, _) is a no-op.",
        c_expect: "C: array of 31 register slots plus PC; index 31 short-circuits to zero constant, same no-op store.",
        lean_log: "rX(31) = 0x0000000000000000; rX(0) after wX = value",
        c_log: "rX(31) = 0x0000000000000000; rX(0) after wX = value",
    },
    SmokeCase {
        id: "range-narrow",
        sail_source: "range.sail",
        sail_function: "f_int(x : range(0,31))",
        description: "Execute int-range narrowing: range(0,31) -> range(-2,2) returning constant 0.",
        lean_expect: "Lean: `Nat` argument refined to `{x // x <= 31}`; result `0 : {x // -2 <= x ∧ x <= 2}` typechecks.",
        c_expect: "C: `int` with runtime range assertion on entry; same return 0, same out-of-range abort behaviour.",
        lean_log: "f_int(31) = 0 in-range",
        c_log: "f_int(31) = 0 in-range",
    },
    SmokeCase {
        id: "loop-sum",
        sail_source: "loop.sail",
        sail_function: "foreach_loopuseindex(m, n)",
        description: "Execute int accumulation loop: res += i over foreach i from m to n.",
        lean_expect: "Lean: `foreach` lowers to fuel-bounded recursion; foreach_loopuseindex(0,3) = 6.",
        c_expect: "C: plain `for` loop with `uint64` accumulator; same sum 6, same empty-range (m>n) yields 0.",
        lean_log: "foreach_loopuseindex(0, 3) = 6",
        c_log: "foreach_loopuseindex(0, 3) = 6",
    },
];

/// Outcome of comparing two log snippets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Comparison {
    Match,
    Mismatch { detail: MismatchDetail },
}

/// Machine-readable mismatch detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MismatchDetail {
    pub case_id: &'static str,
    pub lean_normalized: String,
    pub c_normalized: String,
}

impl fmt::Display for Comparison {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Comparison::Match => write!(f, "match"),
            Comparison::Mismatch { detail } => write!(
                f,
                "mismatch[{}]: lean={:?} c={:?}",
                detail.case_id, detail.lean_normalized, detail.c_normalized
            ),
        }
    }
}

/// Normalize a log snippet: CRLF -> LF, trim lines, drop blank lines,
/// collapse inner whitespace runs to a single space.
pub fn normalize_log(log: &str) -> String {
    let unified = log.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = Vec::new();
    for line in unified.lines() {
        let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.is_empty() {
            continue;
        }
        out.push(collapsed);
    }
    out.join("\n")
}

/// Compare Lean vs C log snippets for one case.
pub fn compare_logs(case: &SmokeCase, lean_log: &str, c_log: &str) -> Comparison {
    let lean_normalized = normalize_log(lean_log);
    let c_normalized = normalize_log(c_log);
    if lean_normalized == c_normalized {
        Comparison::Match
    } else {
        Comparison::Mismatch {
            detail: MismatchDetail {
                case_id: case.id,
                lean_normalized,
                c_normalized,
            },
        }
    }
}

/// Compare the stored fixture logs for one case.
pub fn run_case(case: &SmokeCase) -> Comparison {
    compare_logs(case, case.lean_log, case.c_log)
}

/// Result of running the whole fixture table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiteResult {
    pub matched: usize,
    pub mismatched: usize,
    pub mismatches: Vec<MismatchDetail>,
}

/// Run all fixtures against their stored log snippets.
pub fn run_all() -> SuiteResult {
    let mut mismatches = Vec::new();
    for case in SMOKE_CASES.iter() {
        if let Comparison::Mismatch { detail } = run_case(case) {
            mismatches.push(detail);
        }
    }
    SuiteResult {
        matched: SMOKE_CASES.len() - mismatches.len(),
        mismatched: mismatches.len(),
        mismatches,
    }
}

/// Look up a fixture by id.
pub fn find_case(id: &str) -> Option<&'static SmokeCase> {
    SMOKE_CASES.iter().find(|c| c.id == id)
}

/// Runner sketch for later binary-vs-binary comparison.
///
/// This does not execute anything yet: it records which two binaries would
/// be invoked per case and how their stdout snippets would be fed into
/// [`compare_logs`]. The executor (added in a follow-up) will run
/// `lean_bin` and `c_bin` with `args`, capture stdout, and compare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerPlan {
    pub case_id: &'static str,
    pub lean_bin: String,
    pub c_bin: String,
    pub args: Vec<String>,
}

/// Configuration for the future two-binary runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerConfig {
    /// Path to the Lean-backend binary (Rust-Lean output runner).
    pub lean_bin: String,
    /// Path to the C-backend binary (OCaml-C output binary).
    pub c_bin: String,
}

impl RunnerConfig {
    pub fn new(lean_bin: &str, c_bin: &str) -> Self {
        Self {
            lean_bin: lean_bin.to_string(),
            c_bin: c_bin.to_string(),
        }
    }

    /// Build a per-case plan: same args for both binaries, comparison
    /// happens later via [`compare_logs`].
    pub fn plan_for(&self, case: &SmokeCase, args: &[&str]) -> RunnerPlan {
        RunnerPlan {
            case_id: case.id,
            lean_bin: self.lean_bin.clone(),
            c_bin: self.c_bin.clone(),
            args: args.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Build plans for all fixtures with the same args.
    pub fn plan_all(&self, args: &[&str]) -> Vec<RunnerPlan> {
        SMOKE_CASES.iter().map(|c| self.plan_for(c, args)).collect()
    }
}

#[cfg(test)]
mod tests;
