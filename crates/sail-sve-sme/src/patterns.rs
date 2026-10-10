//! Vector-pattern/loop catalog and shard-stage fail list for SVE/SME shards.
//!
//! Analysis runs on the *source slice* of each definition (via its
//! [`crate::span::Span`], never on copied bodies): one comment- and
//! string-aware token pass per definition, so peak memory stays proportional
//! to a single definition plus the hit/fail records.
//!
//! * [`PatternKind`] — vector/loop idioms prioritized by this lane
//!   (`foreach`, predicated execution, element/mask/register accessors,
//!   VL dispatch, ...), each hit carrying an exact [`crate::span::Span`].
//! * [`FailKind`] — constructs the shard stage cannot lower (KEIN Full-Build
//!   in this lane), each with the owning lane that must handle it.
//!   Fail counts are *sites*, not definitions.

use crate::mini_ast::{Def, ParsedFile};
use crate::span::{ParseError, Span};
use std::collections::BTreeMap;

/// A vector/loop idiom found in a shard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PatternKind {
    /// `foreach (... in inc)`.
    ForeachInc,
    /// `foreach (... in dec)` (reverse, rare: 12 sites in SVE).
    ForeachDec,
    /// `foreach` without a recognizable `in inc/dec` direction.
    ForeachUnknownDir,
    /// A definition body containing 2+ `foreach` loops (e.g. SME 2-D tiles).
    NestedForeach,
    /// C-like `for (...)` loop (absent from both shards; kept for forward compat).
    ForLoop,
    /// `while ... do` loop (1 site in SVE).
    WhileLoop,
    /// `Elem_read(...)` element extraction.
    ElemRead,
    /// `Elem_set(...)` element insertion.
    ElemSet,
    /// `ActivePredicateElement(...)` predicated-lane guard.
    PredicatedExec,
    /// `P_read(...)` predicate-register read.
    MaskRead,
    /// `P_set(...)` predicate-register write.
    MaskWrite,
    /// `Z_read(...)` vector-register read.
    VecRegRead,
    /// `Z_set(...)` vector-register write.
    VecRegWrite,
    /// `ZAtile_*` / other `ZA*` tile accessors (SME).
    TileAccess,
    /// Any other `CamelCase(...)` helper call (needs Lane-25 FFI mapping).
    ExternHelper,
    /// `match VL { 128 => ... }` vector-length dispatch.
    VlDispatch,
    /// `assert(constraint(...))` refinement assumption.
    ConstraintAssert,
    /// `assert(...)` without `constraint` (in practice `assert(false)` markers).
    AssertGeneric,
    /// `throw(Error_...)` trap.
    ThrowUndef,
    /// `= undefined` uninitialized value.
    UndefinedInit,
    /// `Check*Enabled(...)` architecture-guard call.
    CheckEnabled,
    /// `CurrentVL_read()` / `HaveSVE()` / `HaveSME()` queries.
    VlQuery,
    /// `val` signature with `forall`/`constraint`/`is_VL` dependent types.
    DependentSig,
}

impl PatternKind {
    /// All variants, in stable report order.
    pub fn all() -> &'static [PatternKind] {
        use PatternKind::*;
        &[
            ForeachInc,
            ForeachDec,
            ForeachUnknownDir,
            NestedForeach,
            ForLoop,
            WhileLoop,
            ElemRead,
            ElemSet,
            PredicatedExec,
            MaskRead,
            MaskWrite,
            VecRegRead,
            VecRegWrite,
            TileAccess,
            ExternHelper,
            VlDispatch,
            ConstraintAssert,
            AssertGeneric,
            ThrowUndef,
            UndefinedInit,
            CheckEnabled,
            VlQuery,
            DependentSig,
        ]
    }

    pub fn as_str(self) -> &'static str {
        use PatternKind::*;
        match self {
            ForeachInc => "foreach_inc",
            ForeachDec => "foreach_dec",
            ForeachUnknownDir => "foreach_unknown_dir",
            NestedForeach => "nested_foreach",
            ForLoop => "for_loop",
            WhileLoop => "while_loop",
            ElemRead => "elem_read",
            ElemSet => "elem_set",
            PredicatedExec => "predicated_exec",
            MaskRead => "mask_read",
            MaskWrite => "mask_write",
            VecRegRead => "vec_reg_read",
            VecRegWrite => "vec_reg_write",
            TileAccess => "tile_access",
            ExternHelper => "extern_helper",
            VlDispatch => "vl_dispatch",
            ConstraintAssert => "constraint_assert",
            AssertGeneric => "assert_generic",
            ThrowUndef => "throw",
            UndefinedInit => "undefined_init",
            CheckEnabled => "check_enabled",
            VlQuery => "vl_query",
            DependentSig => "dependent_sig",
        }
    }
}

/// A construct the shard stage cannot lower, with the owning lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FailKind {
    /// Loops need Lane-27 lowering (`sail-loops`).
    LoopLowering,
    /// `constraint`/`forall`/`is_VL` need Lane-04/05 (nexp + typecheck).
    ConstraintSolving,
    /// `throw`/`assert` need Lane-28 (early-return/exception semantics).
    ExceptionSemantics,
    /// Helper calls need Lane-25 (extern/FFI mapping).
    ExternMapping,
    /// `match VL` dispatch needs Lane-20/21 (mapping + exhaustiveness).
    VlDispatch,
    /// `undefined` needs Lane-09/22 (lowering + bitvector semantics).
    UndefinedInit,
}

impl FailKind {
    pub fn all() -> &'static [FailKind] {
        use FailKind::*;
        &[
            LoopLowering,
            ConstraintSolving,
            ExceptionSemantics,
            ExternMapping,
            VlDispatch,
            UndefinedInit,
        ]
    }

    pub fn as_str(self) -> &'static str {
        use FailKind::*;
        match self {
            LoopLowering => "loop_lowering",
            ConstraintSolving => "constraint_solving",
            ExceptionSemantics => "exception_semantics",
            ExternMapping => "extern_mapping",
            VlDispatch => "vl_dispatch",
            UndefinedInit => "undefined_init",
        }
    }

    /// Owning lane(s) in `lanes/` numbering.
    pub fn lane(self) -> &'static str {
        use FailKind::*;
        match self {
            LoopLowering => "27 (loops)",
            ConstraintSolving => "04/05 (nexp-constraint/typechecker)",
            ExceptionSemantics => "28 (early-return-except)",
            ExternMapping => "25 (extern-ffi-mapping)",
            VlDispatch => "20/21 (mapping-encdec/pattern-exhaustiveness)",
            UndefinedInit => "09/22 (mono-anf-lowering/bitvector-semantics)",
        }
    }
}

/// One pattern occurrence with its exact source span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub kind: PatternKind,
    pub span: Span,
    /// Name of the enclosing `val`/`function` definition.
    pub def_name: String,
}

/// One non-lowerable site with its exact source span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fail {
    pub kind: FailKind,
    pub span: Span,
    /// Name of the enclosing `val`/`function` definition.
    pub def_name: String,
    /// Matched source text plus what is missing.
    pub detail: String,
}

/// Analysis result for one shard file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReport {
    pub path: String,
    pub bytes: usize,
    pub lines: u64,
    pub def_total: usize,
    pub val_count: usize,
    pub fun_count: usize,
    pub other_count: usize,
    pub pattern_counts: BTreeMap<&'static str, u64>,
    pub fail_counts: BTreeMap<&'static str, u64>,
    pub hits: Vec<Hit>,
    pub fails: Vec<Fail>,
}

impl FileReport {
    pub fn pattern_count(&self, kind: PatternKind) -> u64 {
        self.pattern_counts.get(kind.as_str()).copied().unwrap_or(0)
    }

    pub fn fail_count(&self, kind: FailKind) -> u64 {
        self.fail_counts.get(kind.as_str()).copied().unwrap_or(0)
    }

    pub fn total_fails(&self) -> u64 {
        self.fail_counts.values().sum()
    }
}

/// Byte offset of every line start; maps absolute byte offsets to
/// 1-based (line, col). Columns count bytes, matching the vendored parser
/// convention (both shards are ASCII, so byte cols == char cols).
#[derive(Debug, Clone)]
struct LineTable {
    starts: Vec<usize>,
}

impl LineTable {
    fn new(src: &str) -> Self {
        let bytes = src.as_bytes();
        let mut starts = Vec::with_capacity(volving_estimate(src.len()));
        starts.push(0);
        for (i, b) in bytes.iter().enumerate() {
            if *b == b'\n' && i + 1 < bytes.len() {
                starts.push(i + 1);
            }
        }
        LineTable { starts }
    }

    /// 1-based (line, col) for an absolute byte offset.
    fn line_col(&self, off: usize) -> (u32, u32) {
        let idx = self.starts.partition_point(|s| *s <= off);
        let line_idx = idx.saturating_sub(1);
        let line = (line_idx + 1) as u32;
        let col = (off.saturating_sub(self.starts[line_idx]) + 1) as u32;
        (line, col)
    }

    fn span(&self, file: &str, start: usize, end: usize) -> Span {
        let (sl, sc) = self.line_col(start);
        let (el, ec) = self.line_col(end);
        Span::new(file, start, end, sl, sc, el, ec)
    }

    fn line_count(&self) -> u64 {
        self.starts.len() as u64
    }
}

fn volving_estimate(len: usize) -> usize {
    // ~40 bytes/line average for these shards; over-allocating slightly is
    // cheaper than re-allocating a 138k-entry table.
    (len / 32).max(16)
}

/// One lexical token with offsets relative to the scanned slice.
#[derive(Debug, Clone, Copy)]
struct Tok<'a> {
    text: &'a str,
    start: usize,
    end: usize,
    is_word: bool,
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'\''
}

/// Comment- and string-aware token pass over one definition slice.
/// Skips whitespace, `//` line comments, nested `/* */` block comments,
/// `"..."` strings and `'x'` char literals so hits never come from dead text.
fn tokenize(slice: &str) -> Vec<Tok<'_>> {
    let b = slice.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b' ' || c == b'\t' || c == b'\r' || c == b'\n' {
            i += 1;
        } else if c == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            let mut depth: u32 = 1;
            i += 2;
            while i < b.len() && depth > 0 {
                if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if i + 1 < b.len() && b[i] == b'*' && b[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        } else if c == b'"' {
            i += 1;
            while i < b.len() {
                if b[i] == b'\\' {
                    i += 2;
                } else if b[i] == b'"' {
                    i += 1;
                    break;
                } else if b[i] == b'\n' {
                    break;
                } else {
                    i += 1;
                }
            }
        } else if c == b'\'' && i + 2 < b.len() && b[i + 2] == b'\'' {
            i += 3;
        } else if is_ident_start(c) {
            let start = i;
            while i < b.len() && is_ident_char(b[i]) {
                i += 1;
            }
            toks.push(Tok { text: &slice[start..i], start, end: i, is_word: true });
        } else {
            toks.push(Tok { text: &slice[i..i + 1], start: i, end: i + 1, is_word: false });
            i += 1;
        }
    }
    toks
}

/// Next word token at/after `from` (skips punctuation).
fn next_word<'a>(toks: &'a [Tok<'a>], from: usize) -> Option<(usize, &'a Tok<'a>)> {
    toks.iter().enumerate().skip(from).find(|(_, t)| t.is_word)
}

/// True for `CamelCase` helper names (first char uppercase + contains a
/// lowercase letter): `UInt`, `Elem_read`, `CheckSVEEnabled`, `ZAtile_read`.
/// All-caps names (`VL`, `FADD_Z_P_ZS__`, `DIV`) and lowercase keywords are
/// excluded; a few capitalized non-calls are excluded explicitly.
fn is_extern_helper_call(name: &str) -> bool {
    if matches!(name, "Bool" | "True" | "False" | "Int" | "Bits") {
        return false;
    }
    let mut bytes = name.bytes();
    match bytes.next() {
        Some(c) if c.is_ascii_uppercase() => (),
        _ => return false,
    }
    if name.bytes().any(|c| c.is_ascii_lowercase()) {
        return true;
    }
    false
}

fn is_check_enabled(name: &str) -> bool {
    name.starts_with("Check") && name.ends_with("Enabled")
}

fn is_vl_query(name: &str) -> bool {
    matches!(name, "CurrentVL_read" | "HaveSVE" | "HaveSME" | "HaveSVEFP" | "HaveSVEB")
        || name.starts_with("Have")
}

fn def_name(def: &Def) -> String {
    match def {
        Def::Val { name, .. } => name.clone(),
        Def::Fundef { name, .. } => name.clone(),
        Def::Mapdef { name, .. } => name.clone(),
        Def::Impl { name, .. } => name.clone(),
        Def::Type { name, .. } => name.clone(),
        Def::Overload { name, .. } => name.clone(),
        Def::Register { name, .. } => name.clone(),
        Def::Fixity { name, .. } => name.clone(),
        Def::Pragma { name, .. } => format!("${name}"),
        Def::Other { keyword, .. } => keyword.clone(),
        _ => "?".to_owned(),
    }
}

fn is_val_def(def: &Def) -> bool {
    matches!(def, Def::Val { .. })
}

struct Ctx<'a> {
    file: &'a str,
    lines: &'a LineTable,
    report: FileReport,
}

impl Ctx<'_> {
    fn push_hit(&mut self, kind: PatternKind, def_span_start: usize, tok: &Tok<'_>, def: &str) {
        let span = self.lines.span(self.file, def_span_start + tok.start, def_span_start + tok.end);
        *self.report.pattern_counts.entry(kind.as_str()).or_insert(0) += 1;
        self.report.hits.push(Hit { kind, span, def_name: def.to_owned() });
    }

    fn push_hit_span(
        &mut self,
        kind: PatternKind,
        def_span_start: usize,
        start: usize,
        end: usize,
        def: &str,
    ) {
        let span = self.lines.span(self.file, def_span_start + start, def_span_start + end);
        *self.report.pattern_counts.entry(kind.as_str()).or_insert(0) += 1;
        self.report.hits.push(Hit { kind, span, def_name: def.to_owned() });
    }

    fn push_fail(
        &mut self,
        kind: FailKind,
        def_span_start: usize,
        start: usize,
        end: usize,
        def: &str,
        detail: &str,
    ) {
        let span = self.lines.span(self.file, def_span_start + start, def_span_start + end);
        *self.report.fail_counts.entry(kind.as_str()).or_insert(0) += 1;
        self.report.fails.push(Fail {
            kind,
            span,
            def_name: def.to_owned(),
            detail: detail.to_owned(),
        });
    }
}

/// Analyze one already-parsed file. `src` must be the exact input `parsed`
/// was built from (spans index into it); it is only borrowed, never copied.
pub fn analyze_parsed(path: &str, src: &str, parsed: &ParsedFile) -> FileReport {
    let lines = LineTable::new(src);
    let line_count = lines.line_count();
    let mut ctx = Ctx {
        file: path,
        lines: &lines,
        report: FileReport {
            path: path.to_owned(),
            bytes: src.len(),
            lines: line_count,
            def_total: 0,
            val_count: 0,
            fun_count: 0,
            other_count: 0,
            pattern_counts: BTreeMap::new(),
            fail_counts: BTreeMap::new(),
            hits: Vec::new(),
            fails: Vec::new(),
        },
    };

    for sd in &parsed.defs {
        // Shard markers carry no analyzable source.
        if sd.def.is_shard_marker() {
            continue;
        }
        ctx.report.def_total += 1;
        match &sd.def {
            Def::Val { .. } => ctx.report.val_count += 1,
            Def::Fundef { .. } => ctx.report.fun_count += 1,
            _ => ctx.report.other_count += 1,
        }
        let name = def_name(&sd.def);
        let base = sd.span.start_byte;
        let end = sd.span.end_byte.min(src.len());
        if base >= end || base >= src.len() {
            continue;
        }
        let slice = &src[base..end];
        analyze_def(&mut ctx, &sd.def, &name, base, slice);
    }
    ctx.report
}

fn analyze_def(ctx: &mut Ctx<'_>, def: &Def, name: &str, base: usize, slice: &str) {
    let toks = tokenize(slice);
    let is_val = is_val_def(def);
    let mut foreach_count: u32 = 0;

    let mut i = 0;
    while i < toks.len() {
        let t = &toks[i];
        if !t.is_word {
            i += 1;
            continue;
        }
        match t.text {
            "foreach" => {
                foreach_count += 1;
                // Direction: `in inc` / `in dec` within the header.
                let mut dir = "unknown";
                let mut j = i + 1;
                let mut seen = 0;
                while j < toks.len() && seen < 16 {
                    if toks[j].is_word {
                        seen += 1;
                        if toks[j].text == "in" {
                            if let Some((_, nt)) = next_word(&toks, j + 1) {
                                if nt.text == "inc" {
                                    dir = "inc";
                                } else if nt.text == "dec" {
                                    dir = "dec";
                                }
                            }
                            break;
                        }
                        if toks[j].text == "foreach" {
                            break;
                        }
                    } else if toks[j].text == "{" {
                        break;
                    }
                    j += 1;
                }
                let (kind, detail) = match dir {
                    "inc" => (
                        PatternKind::ForeachInc,
                        "foreach ... in inc: element loop needs Lane-27 lowering",
                    ),
                    "dec" => (
                        PatternKind::ForeachDec,
                        "foreach ... in dec: reverse loop needs Lane-27 lowering",
                    ),
                    _ => (
                        PatternKind::ForeachUnknownDir,
                        "foreach without in inc/dec: needs Lane-27 lowering",
                    ),
                };
                let (start, end) = (t.start, t.end);
                ctx.push_hit(kind, base, t, name);
                ctx.push_fail(FailKind::LoopLowering, base, start, end, name, detail);
                if foreach_count == 2 {
                    ctx.push_hit_span(PatternKind::NestedForeach, base, start, end, name);
                }
            }
            "for" => {
                let (start, end) = (t.start, t.end);
                ctx.push_hit(PatternKind::ForLoop, base, t, name);
                ctx.push_fail(
                    FailKind::LoopLowering,
                    base,
                    start,
                    end,
                    name,
                    "for loop: needs Lane-27 lowering",
                );
            }
            "while" => {
                let (start, end) = (t.start, t.end);
                ctx.push_hit(PatternKind::WhileLoop, base, t, name);
                ctx.push_fail(
                    FailKind::LoopLowering,
                    base,
                    start,
                    end,
                    name,
                    "while loop: needs Lane-27 lowering + termination measure",
                );
            }
            "Elem_read" => {
                let (start, end) = (t.start, t.end);
                ctx.push_hit(PatternKind::ElemRead, base, t, name);
                ctx.push_fail(
                    FailKind::ExternMapping,
                    base,
                    start,
                    end,
                    name,
                    "Elem_read: element accessor needs Lane-25 FFI mapping",
                );
            }
            "Elem_set" => {
                let (start, end) = (t.start, t.end);
                ctx.push_hit(PatternKind::ElemSet, base, t, name);
                ctx.push_fail(
                    FailKind::ExternMapping,
                    base,
                    start,
                    end,
                    name,
                    "Elem_set: element insert needs Lane-25 FFI mapping",
                );
            }
            "ActivePredicateElement" => {
                let (start, end) = (t.start, t.end);
                ctx.push_hit(PatternKind::PredicatedExec, base, t, name);
                ctx.push_fail(
                    FailKind::ExternMapping,
                    base,
                    start,
                    end,
                    name,
                    "ActivePredicateElement: predicated-lane guard needs Lane-25 mapping",
                );
            }
            "P_read" => {
                let (start, end) = (t.start, t.end);
                ctx.push_hit(PatternKind::MaskRead, base, t, name);
                ctx.push_fail(
                    FailKind::ExternMapping,
                    base,
                    start,
                    end,
                    name,
                    "P_read: predicate-register read needs Lane-25 mapping",
                );
            }
            "P_set" => {
                let (start, end) = (t.start, t.end);
                ctx.push_hit(PatternKind::MaskWrite, base, t, name);
                ctx.push_fail(
                    FailKind::ExternMapping,
                    base,
                    start,
                    end,
                    name,
                    "P_set: predicate-register write needs Lane-25 mapping",
                );
            }
            "Z_read" => {
                let (start, end) = (t.start, t.end);
                ctx.push_hit(PatternKind::VecRegRead, base, t, name);
                ctx.push_fail(
                    FailKind::ExternMapping,
                    base,
                    start,
                    end,
                    name,
                    "Z_read: vector-register read needs Lane-25 mapping",
                );
            }
            "Z_set" => {
                let (start, end) = (t.start, t.end);
                ctx.push_hit(PatternKind::VecRegWrite, base, t, name);
                ctx.push_fail(
                    FailKind::ExternMapping,
                    base,
                    start,
                    end,
                    name,
                    "Z_set: vector-register write needs Lane-25 mapping",
                );
            }
            "match" => {
                if let Some((_, nt)) = next_word(&toks, i + 1) {
                    if nt.text == "VL" {
                        ctx.push_hit_span(
                            PatternKind::VlDispatch,
                            base,
                            t.start,
                            nt.end,
                            name,
                        );
                        ctx.push_fail(
                            FailKind::VlDispatch,
                            base,
                            t.start,
                            nt.end,
                            name,
                            "match VL: vector-length dispatch needs Lane-20/21",
                        );
                    }
                }
            }
            "assert" => {
                // `assert(constraint(...))` vs `assert(false)` etc.
                let is_paren = matches!(toks.get(i + 1),
                    Some(nt) if !nt.is_word && nt.text == "(");
                let constrained = if is_paren {
                    matches!(next_word(&toks, i + 2), Some((_, nt)) if nt.text == "constraint")
                } else {
                    false
                };
                // Span covers `assert` .. closing of the constraint word when present.
                let (start, end) = (t.start, t.end);
                if constrained {
                    ctx.push_hit(PatternKind::ConstraintAssert, base, t, name);
                    ctx.push_fail(
                        FailKind::ConstraintSolving,
                        base,
                        start,
                        end,
                        name,
                        "assert(constraint(...)): refinement needs Lane-04/05",
                    );
                } else {
                    ctx.push_hit(PatternKind::AssertGeneric, base, t, name);
                    ctx.push_fail(
                        FailKind::ExceptionSemantics,
                        base,
                        start,
                        end,
                        name,
                        "assert(...): runtime check needs Lane-28 semantics",
                    );
                }
            }
            "throw" => {
                let (start, end) = (t.start, t.end);
                ctx.push_hit(PatternKind::ThrowUndef, base, t, name);
                ctx.push_fail(
                    FailKind::ExceptionSemantics,
                    base,
                    start,
                    end,
                    name,
                    "throw(...): trap needs Lane-28 semantics",
                );
            }
            "undefined" => {
                let (start, end) = (t.start, t.end);
                ctx.push_hit(PatternKind::UndefinedInit, base, t, name);
                ctx.push_fail(
                    FailKind::UndefinedInit,
                    base,
                    start,
                    end,
                    name,
                    "= undefined: uninitialized value needs Lane-09/22",
                );
            }
            "forall" | "constraint" | "is_VL" if is_val => {
                let (start, end) = (t.start, t.end);
                ctx.push_hit(PatternKind::DependentSig, base, t, name);
                ctx.push_fail(
                    FailKind::ConstraintSolving,
                    base,
                    start,
                    end,
                    name,
                    "dependent val signature: needs Lane-04/05 solving",
                );
            }
            w => {
                if is_check_enabled(w) {
                    let (start, end) = (t.start, t.end);
                    ctx.push_hit(PatternKind::CheckEnabled, base, t, name);
                    ctx.push_fail(
                        FailKind::ExternMapping,
                        base,
                        start,
                        end,
                        name,
                        "Check*Enabled: arch guard needs Lane-25 mapping",
                    );
                } else if is_vl_query(w) {
                    let (start, end) = (t.start, t.end);
                    ctx.push_hit(PatternKind::VlQuery, base, t, name);
                    ctx.push_fail(
                        FailKind::ExternMapping,
                        base,
                        start,
                        end,
                        name,
                        "VL query (CurrentVL/Have*): needs Lane-25 mapping",
                    );
                } else if w.starts_with("ZAtile_") || w.starts_with("ZA") && w.len() > 2 {
                    // SME tile accessors (`ZAtile_read`, ...); plain `ZA`
                    // alone is not a call and is ignored.
                    let followed_by_call = matches!(toks.get(i + 1),
                        Some(nt) if !nt.is_word && nt.text == "(");
                    if followed_by_call || w.starts_with("ZAtile_") {
                        let (start, end) = (t.start, t.end);
                        ctx.push_hit(PatternKind::TileAccess, base, t, name);
                        ctx.push_fail(
                            FailKind::ExternMapping,
                            base,
                            start,
                            end,
                            name,
                            "SME tile accessor: needs Lane-25 mapping",
                        );
                    }
                } else if is_extern_helper_call(w) {
                    // Generic CamelCase helper call — but not the callee of
                    // `throw(...)` and not a definition header name.
                    let prev_is_throw = i > 0
                        && toks[i - 1].is_word
                        && toks[i - 1].text == "throw";
                    // `throw(`: callee is followed by "(", so check one further back.
                    let is_throw_arg = i > 1
                        && toks[i - 2].is_word
                        && toks[i - 2].text == "throw"
                        && !toks[i - 1].is_word
                        && toks[i - 1].text == "(";
                    let followed_by_call = matches!(toks.get(i + 1),
                        Some(nt) if !nt.is_word && nt.text == "(");
                    if followed_by_call && !prev_is_throw && !is_throw_arg {
                        let (start, end) = (t.start, t.end);
                        ctx.push_hit(PatternKind::ExternHelper, base, t, name);
                        ctx.push_fail(
                            FailKind::ExternMapping,
                            base,
                            start,
                            end,
                            name,
                            &format!("{w}(...): helper call needs Lane-25 FFI mapping"),
                        );
                    }
                }
            }
        }
        i += 1;
    }
}

/// Parse one shard file and analyze it in a single pass.
pub fn analyze_file(path: &str, src: &str) -> Result<FileReport, ParseError> {
    let parsed = crate::parser::parse_file(path, src)?;
    Ok(analyze_parsed(path, src, &parsed))
}

/// Parse + analyze many shard files in parallel (one thread per file, via
/// [`std::thread::scope`]); results are in input order. Holds no shared
/// mutable state — the intended driver for the SVE+SME shard pair.
pub fn analyze_files_parallel(inputs: &[(&str, &str)]) -> Vec<Result<FileReport, ParseError>> {
    let mut out: Vec<Option<Result<FileReport, ParseError>>> =
        Vec::with_capacity(inputs.len());
    out.resize_with(inputs.len(), || None);
    std::thread::scope(|s| {
        let handles: Vec<_> = inputs
            .iter()
            .enumerate()
            .map(|(i, (path, src))| {
                s.spawn(move || {
                    let r = analyze_file(path, src);
                    (i, r)
                })
            })
            .collect();
        for h in handles {
            let (i, r) = h.join().unwrap_or_else(|_| {
                (
                    usize::MAX,
                    Err(ParseError::new(
                        Span::point("?", 0, 1, 1),
                        "analyzer thread panicked",
                    )),
                )
            });
            if i != usize::MAX {
                out[i] = Some(r);
            }
        }
    });
    out.into_iter().map(|o| o.expect("analyzer thread missing")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_table_basic() {
        let lt = LineTable::new("ab\ncde\nf");
        assert_eq!(lt.line_col(0), (1, 1));
        assert_eq!(lt.line_col(2), (1, 3));
        assert_eq!(lt.line_col(3), (2, 1));
        assert_eq!(lt.line_col(7), (3, 1));
        assert_eq!(lt.line_count(), 3);
    }

    #[test]
    fn tokenizer_skips_comments_and_strings() {
        let toks = tokenize("foreach /* foreach \"foreach\" */ // foreach\n\"foreach\" 'x' foreach");
        let words: Vec<_> =
            toks.iter().filter(|t| t.is_word).map(|t| t.text).collect();
        assert_eq!(words, vec!["foreach", "foreach"]);
    }

    #[test]
    fn tokenizer_emits_punct_and_terminates() {
        // Regression: punctuation branch must advance the cursor (was infinite loop).
        let toks = tokenize("f(x:y, z)");
        let texts: Vec<_> = toks.iter().map(|t| t.text).collect();
        assert_eq!(texts, vec!["f", "(", "x", ":", "y", ",", "z", ")"]);
    }

    #[test]
    fn extern_helper_call_shape() {
        assert!(is_extern_helper_call("Elem_read"));
        assert!(is_extern_helper_call("CheckSVEEnabled"));
        assert!(is_extern_helper_call("UInt"));
        assert!(!is_extern_helper_call("VL"));
        assert!(!is_extern_helper_call("DIV"));
        assert!(!is_extern_helper_call("FADD_Z_P_ZS__"));
        assert!(!is_extern_helper_call("foreach"));
        assert!(!is_extern_helper_call("Bool"));
    }
}
