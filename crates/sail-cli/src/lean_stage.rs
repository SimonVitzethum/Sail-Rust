//! Lean-emit stage: wire rewritten definitions onto Lean text (Lane 64).
//!
//! This module consumes one definition at a time and routes it exactly as
//! the reference backend does:
//! - `Function` / `Value` become Lean `def`s via the [`sail_lean`] printer
//!   core (`print_fun_stub` / `render_fun_stub` — the `emit_def` / `render`
//!   entry points of this worktree; once `sail-lean/src/emit.rs` lands the
//!   call sites switch to `sail_lean::emit::emit_def` / `render_def` with a
//!   `funsig_from_scheme` signature, without changing the routing below).
//! - `Register` becomes a `(constructor, lean_type)` pair for the
//!   [`sail_lake`] register templates (`render_register_inductive` /
//!   `render_register_type`); registers never become `def`s here and the
//!   templates are used unchanged.
//! - `Type` / `Constraint` / `Overload` / `Let` / `Other` are header-only or
//!   out-of-scope forms: they are skipped loudly (verbose log + [`Skipped`]
//!   record) and never emit a placeholder `def`.
//!
//! Failure policy (no stub success, never `sorry`):
//! - every gap — missing signature, missing body, missing register type,
//!   proof hole in a body — surfaces as [`sail_lean::Error::Unsupported`]
//!   carrying its [`Span`](sail_lean::Span) and maps to exit code 1.
//! - sink failures map to exit code 2 (I/O-fatal).
//! - bodies are scanned for proof holes (`sorry` / `admit` as Lean tokens,
//!   strings and comments skipped) before anything is written, so `out` is
//!   untouched on that path; a `None` body is refused the same way instead
//!   of emitting a silent `sorry`.
//!
//! ## MAIN-RS-INSERT-SPEC (for the `lean-emit` wiring owner; `main.rs` is NOT
//! touched by this lane)
//!
//! ```ignore
//! // 0. In crates/sail-cli/src/main.rs, declare the module:
//! mod lean_stage;
//!
//! // 1. In run_compiler, after the rewrite stage yields `rewritten`
//! //    (rewrite_stage::run over Checked::defs, source order):
//! let lean_out = profiler.phase("lean-emit", || {
//!     // 2. Lower each rewritten def to a lean_stage::LeanStageDef
//!     //    (1:1 kind map: Function | Value | Register | Type |
//!     //    Constraint | Overload | Let | Other):
//!     //    - Function | Value => {
//!     //        let typed = &checked.defs[position]; // scheme owner
//!     //        let scheme = typed.scheme().ok_or(missing_scheme)?; // loud
//!     //        let it = typed.interner().ok_or(missing_interner)?; // loud
//!     //        // binder names from the loaded clause patterns
//!     //        // (pats::pat_from_ast binders / clause PExp), effect flags
//!     //        // from effect analysis over the clause body:
//!     //        let sig = sail_lean::emit::funsig_from_scheme(
//!     //            &name, emit_kind, scheme, &param_names, flags, span, it)?;
//!     //        // body text via exps.rs (LeanPrinter::print_exp_top with
//!     //        // ExpCtx), then:
//!     //        LeanStageDef::function(name, position, sig, Some(body_text))
//!     //      }
//!     //    - Register => {
//!     //        let (ctor, ty) = sail_lean::emit::register_entry(
//!     //            &printer, &name, scheme, span, it)?;
//!     //        LeanStageDef::register(name, position, span, lean_ty_of(&ty))
//!     //      }
//!     //    - Type | Constraint | Overload | Let | Other => {
//!     //        LeanStageDef::skipped(name, position, kind) // loud log, no def
//!     //      }
//!     // 3. Stream the defs out (peak RAM stays O(largest single def)):
//!     let mut out = String::new(); // or the shard file buffer
//!     let outcome = lean_stage::run_into(&printer, &stage_defs, &mut out)?;
//!     // 4. Feed outcome.registers into the unchanged sail-lake templates:
//!     //    render_register_inductive / render_register_type
//!     //    (empty model => render_empty_registers).
//!     // 5. Every `?` above is Error::Unsupported with Span => exit 1.
//!     //    No path may write `sorry` / `admit`.
//!     lean_stage::render_register_section(&outcome.registers)
//! })?;
//! ```
//!
//! The elaborated `scripts/CheckLeanNoSorry.lean` gate stays authoritative
//! for proof holes in produced files; [`body_has_hole`] here is
//! emission-time defense in depth.

#![forbid(unsafe_code)]

use sail_lean::{Error as LeanError, FunSig, LeanPrinter, Span, Typ};
use sail_lake::RegisterEntry;

use crate::args::{CliError, EXIT_ERROR};

/// Which declaration form a stage input was elaborated from.
///
/// 1:1 mirror of the reference `TypedDefKind` (`Function`, `Value`,
/// `Register`, `Type`, `Constraint`, `Overload`, `Let`, `Other`), so the
/// future `Checked::defs`/`Rewritten` adapter maps each variant verbatim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeanDefKind {
    Function,
    Value,
    Register,
    Type,
    Constraint,
    Overload,
    Let,
    Other,
}

impl LeanDefKind {
    /// Short label for logs and error messages.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Value => "value",
            Self::Register => "register",
            Self::Type => "type",
            Self::Constraint => "constraint",
            Self::Overload => "overload",
            Self::Let => "let",
            Self::Other => "other",
        }
    }

    /// Whether this kind becomes a Lean `def`.
    #[must_use]
    pub fn is_value_like(self) -> bool {
        matches!(self, Self::Function | Self::Value)
    }
}

/// One stage input: an already-lowered definition plus its routing kind.
///
/// Fixture form of a rewritten typed definition: `sig`/`body` are the
/// printer-ready lowering (signature via `funsig_from_scheme` plus body text
/// via `exps.rs` in the wired driver), `register_ty` the lowered register
/// type. `position` is the source-order manifest index for log correlation.
#[derive(Clone, Debug)]
pub struct LeanStageDef {
    pub name: String,
    pub kind: LeanDefKind,
    pub position: usize,
    pub span: Span,
    pub sig: Option<FunSig>,
    pub body: Option<String>,
    pub register_ty: Option<Typ>,
}

impl LeanStageDef {
    /// Value-like def with printer-ready signature and body text.
    /// `body: None` models "body not translatable in this lane" and is a
    /// hard error at emission, never a silent `sorry`.
    #[must_use]
    pub fn function(name: String, position: usize, sig: FunSig, body: Option<String>) -> Self {
        Self {
            span: sig.span,
            name,
            kind: LeanDefKind::Function,
            position,
            sig: Some(sig),
            body,
            register_ty: None,
        }
    }

    /// Same as [`Self::function`] for `val` values.
    #[must_use]
    pub fn value(name: String, position: usize, sig: FunSig, body: Option<String>) -> Self {
        Self {
            span: sig.span,
            name,
            kind: LeanDefKind::Value,
            position,
            sig: Some(sig),
            body,
            register_ty: None,
        }
    }

    /// Register declaration: feeds the `sail-lake` templates, never a `def`.
    #[must_use]
    pub fn register(name: String, position: usize, span: Span, ty: Typ) -> Self {
        Self {
            name,
            kind: LeanDefKind::Register,
            position,
            span,
            sig: None,
            body: None,
            register_ty: Some(ty),
        }
    }

    /// Header-only / out-of-scope form: skipped loudly, no placeholder `def`.
    #[must_use]
    pub fn skipped(name: String, kind: LeanDefKind, position: usize, span: Span) -> Self {
        debug_assert!(!kind.is_value_like() && !matches!(kind, LeanDefKind::Register));
        Self {
            name,
            kind,
            position,
            span,
            sig: None,
            body: None,
            register_ty: None,
        }
    }
}

/// A skipped header-only / out-of-scope form (log record, no Lean output).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub position: usize,
    pub name: String,
    pub kind: &'static str,
}

/// Outcome of one lean-emit run over stage inputs.
///
/// `definitions` is the input count (derived observation, never a success
/// criterion); `emitted` counts written `def`s; `registers` feeds the
/// unchanged `sail-lake` templates; `skipped` records every loud skip.
#[derive(Clone, Debug, Default)]
pub struct LeanOutcome {
    pub definitions: usize,
    pub emitted: usize,
    pub registers: Vec<RegisterEntry>,
    pub skipped: Vec<Skipped>,
}

impl LeanOutcome {
    /// Whether nothing was emitted and no registers were collected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.emitted == 0 && self.registers.is_empty()
    }
}

fn error(message: impl Into<String>) -> CliError {
    CliError {
        exit_code: EXIT_ERROR,
        message: message.into(),
    }
}

fn unsupported(msg: impl Into<String>, span: Span) -> CliError {
    error(format!(
        "sail-rs: lean-emit: cannot translate to Lean at {span}: {}",
        msg.into()
    ))
}

fn lean_error(err: LeanError) -> CliError {
    match err {
        LeanError::Unsupported { msg, span } => unsupported(msg, span),
        LeanError::Fmt(msg) => CliError::fatal(format!("sail-rs: lean-emit: sink failed: {msg}")),
    }
}

/// Whether Lean body text contains a proof hole.
///
/// Token scan over `sorry` / `admit` as standalone Lean identifiers.
/// String literals (`"…"`, Lean `\"` escapes), line comments (`-- …`) and
/// block comments (`/- … -/`, nested) are skipped first, so a Sail string
/// `"sorry"` or a comment mentioning a hole cannot fail a legitimate
/// definition. Emission-time defense in depth; the authoritative check stays
/// the elaborated `scripts/CheckLeanNoSorry.lean` gate.
#[must_use]
pub fn body_has_hole(text: &str) -> bool {
    for token in lean_code_tokens(text) {
        if token == "sorry" || token == "admit" {
            return true;
        }
    }
    false
}

/// Emit one stage input: `def` text, register pair, or loud skip.
///
/// - `Function` / `Value` require a signature; a `None` body or a body with
///   a proof hole is [`LeanError::Unsupported`], and `out` is untouched on
///   those paths (checked before anything is written).
/// - `Register` requires a lowered type and appends to
///   `outcome.registers`; nothing is written to `out`.
/// - the remaining kinds are logged to stderr and recorded in
///   `outcome.skipped`; nothing is written to `out`.
pub fn emit_one<W: std::fmt::Write>(
    printer: &LeanPrinter,
    def: &LeanStageDef,
    out: &mut W,
    outcome: &mut LeanOutcome,
) -> Result<(), CliError> {
    outcome.definitions += 1;
    match def.kind {
        LeanDefKind::Function | LeanDefKind::Value => {
            let sig = def.sig.as_ref().ok_or_else(|| {
                unsupported(
                    format!(
                        "{} `{}` has no lowered signature (no silent sorry)",
                        def.kind.as_str(),
                        def.name
                    ),
                    def.span,
                )
            })?;
            let body = def.body.as_deref().ok_or_else(|| {
                unsupported(
                    format!(
                        "{} `{}` body is not translatable (no silent sorry)",
                        def.kind.as_str(),
                        def.name
                    ),
                    def.span,
                )
            })?;
            if body_has_hole(body) {
                return Err(unsupported(
                    format!(
                        "{} `{}` body contains a proof hole (sorry/admit): refusing to emit",
                        def.kind.as_str(),
                        def.name
                    ),
                    def.span,
                ));
            }
            printer
                .print_fun_stub(out, sig, Some(body))
                .map_err(lean_error)?;
            outcome.emitted += 1;
            Ok(())
        }
        LeanDefKind::Register => {
            let ty = def.register_ty.as_ref().ok_or_else(|| {
                unsupported(
                    format!(
                        "register `{}` has no lowered type (no silent sorry)",
                        def.name
                    ),
                    def.span,
                )
            })?;
            let mut rendered = String::new();
            printer
                .print_typ(&mut rendered, ty)
                .map_err(lean_error)?;
            outcome.registers.push((def.name.clone(), rendered));
            Ok(())
        }
        LeanDefKind::Type
        | LeanDefKind::Constraint
        | LeanDefKind::Overload
        | LeanDefKind::Let
        | LeanDefKind::Other => {
            eprintln!(
                "sail-rs: lean-emit skip {} `{}` at position {} (header-only, no Lean def)",
                def.kind.as_str(),
                def.name,
                def.position,
            );
            outcome.skipped.push(Skipped {
                position: def.position,
                name: def.name.clone(),
                kind: def.kind.as_str(),
            });
            Ok(())
        }
    }
}

/// Stream stage inputs into `out` in order (peak RAM stays O(largest single
/// definition)).
///
/// Stops at the first hard error like the printer core: defs before it are
/// already written, defs after it are not attempted. Every error carries
/// its [`Span`](sail_lean::Span) and maps to exit code 1; nothing on any
/// path writes `sorry` / `admit`.
pub fn run_into<W: std::fmt::Write>(
    printer: &LeanPrinter,
    defs: &[LeanStageDef],
    out: &mut W,
) -> Result<LeanOutcome, CliError> {
    let mut outcome = LeanOutcome::default();
    for def in defs {
        emit_one(printer, def, out, &mut outcome)?;
    }
    Ok(outcome)
}

/// [`run_into`] into a fresh `String` (small per-run buffer owner).
pub fn render_all(
    printer: &LeanPrinter,
    defs: &[LeanStageDef],
) -> Result<(LeanOutcome, String), CliError> {
    let mut out = String::new();
    let outcome = run_into(printer, defs, &mut out)?;
    Ok((outcome, out))
}

/// Render the `Register` inductive section for collected registers through
/// the unchanged `sail-lake` templates (empty model => empty-register
/// fallback).
#[must_use]
pub fn render_register_section(registers: &[RegisterEntry]) -> String {
    if registers.is_empty() {
        return sail_lake::render_empty_registers();
    }
    let mut section = sail_lake::render_register_inductive(registers);
    section.push('\n');
    section.push_str(&sail_lake::render_register_type(registers));
    section
}

/// Split Lean source into code tokens, skipping string literals, `--`
/// line comments and (nested) `/- -/` block comments. Tokens are maximal
/// runs of `[A-Za-z0-9_']`.
fn lean_code_tokens(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    let mut token_start: Option<usize> = None;
    macro_rules! flush {
        ($end:expr) => {
            if let Some(s) = token_start.take() {
                tokens.push(&text[s..$end]);
            }
        };
    }
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            flush!(i);
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            flush!(i);
            let mut depth = 1usize;
            i += 2;
            while i < bytes.len() && depth > 0 {
                if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if c == b'"' {
            flush!(i);
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        if c == b'\'' && char_literal_len(&bytes[i..]).is_some() {
            flush!(i);
            i += char_literal_len(&bytes[i..]).unwrap_or(1);
            continue;
        }
        if c.is_ascii_alphanumeric() || c == b'_' || c == b'\'' {
            if token_start.is_none() {
                token_start = Some(i);
            }
        } else {
            flush!(i);
        }
        i += 1;
    }
    flush!(text.len());
    tokens
}

/// Length of a Lean character literal at the start of `rest` (`'x'`,
/// `'\n'`, `'\\'`, `'\''`), or `None` when it is not one. Used only to
/// keep `'` inside char literals from merging adjacent code tokens.
fn char_literal_len(rest: &[u8]) -> Option<usize> {
    if rest.len() < 3 || rest[0] != b'\'' {
        return None;
    }
    if rest[1] == b'\\' {
        if rest.len() >= 4 && rest[3] == b'\'' {
            return Some(4);
        }
        return None;
    }
    if rest[2] == b'\'' {
        return Some(3);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{FileId, Span};

    fn span() -> Span {
        Span::new(FileId(0), 4, 9)
    }

    fn int_sig(name: &str, args: &[(&str, Typ)]) -> FunSig {
        FunSig {
            name: name.to_owned(),
            args: args
                .iter()
                .map(|(n, t)| ((*n).to_owned(), t.clone()))
                .collect(),
            ret: Typ::Int,
            is_monadic: false,
            early_return: false,
            has_loop: false,
            span: span(),
        }
    }

    /// Fixture-TypedDef: pure `function add(x : Int) : Int`.
    fn fixture_function(position: usize) -> LeanStageDef {
        LeanStageDef::function(
            "add".to_owned(),
            position,
            int_sig("add", &[("x", Typ::Int)]),
            Some("x".to_owned()),
        )
    }

    /// Fixture-TypedDef: plain `val` value `v : Nat`.
    fn fixture_value(position: usize) -> LeanStageDef {
        let mut sig = int_sig("v", &[]);
        sig.ret = Typ::Nat;
        LeanStageDef::value("v".to_owned(), position, sig, Some("42".to_owned()))
    }

    /// Fixture-TypedDef: `register r : Bool`.
    fn fixture_register(position: usize) -> LeanStageDef {
        LeanStageDef::register("r".to_owned(), position, span(), Typ::Bool)
    }

    #[test]
    fn value_like_defs_render_without_sorry() {
        let printer = LeanPrinter::with_default();
        let defs = vec![fixture_function(0), fixture_value(1)];
        let (outcome, out) = render_all(&printer, &defs).unwrap();
        assert_eq!(outcome.definitions, 2);
        assert_eq!(outcome.emitted, 2);
        assert!(outcome.registers.is_empty());
        assert!(outcome.skipped.is_empty());
        assert!(out.contains("def add (x : Int) : Int := x\n"));
        assert!(out.contains("def v : Nat := 42\n"));
        for hole in ["sorry", "admit"] {
            assert!(!out.contains(hole), "output contains {hole:?}:\n{out}");
        }
        assert!(!outcome.is_empty());
    }

    #[test]
    fn emission_preserves_input_order() {
        let printer = LeanPrinter::with_default();
        let defs = vec![fixture_value(1), fixture_function(0)];
        let (outcome, out) = render_all(&printer, &defs).unwrap();
        assert_eq!(outcome.definitions, 2);
        assert!(out.find("def v").unwrap() < out.find("def add").unwrap());
    }

    #[test]
    fn registers_feed_lake_templates_and_never_become_defs() {
        let printer = LeanPrinter::with_default();
        let defs = vec![fixture_register(0), fixture_function(1)];
        let (outcome, out) = render_all(&printer, &defs).unwrap();
        assert_eq!(outcome.definitions, 2);
        assert_eq!(outcome.emitted, 1);
        assert_eq!(
            outcome.registers,
            vec![("r".to_owned(), "Bool".to_owned())]
        );
        assert!(!out.contains("inductive Register"));
        let section = render_register_section(&outcome.registers);
        assert!(section.contains("inductive Register"));
        assert!(section.contains("| r"));
        assert!(section.contains(".r => Bool"));
        assert!(!section.contains("sorry"));
    }

    #[test]
    fn empty_registers_use_the_fallback_template() {
        let section = render_register_section(&[]);
        assert_eq!(section, sail_lake::render_empty_registers());
        assert!(section.contains("PEmpty"));
        assert!(!section.contains("sorry"));
    }

    #[test]
    fn header_only_forms_are_skipped_loudly() {
        let printer = LeanPrinter::with_default();
        let kinds = [
            LeanDefKind::Type,
            LeanDefKind::Constraint,
            LeanDefKind::Overload,
            LeanDefKind::Let,
            LeanDefKind::Other,
        ];
        let defs: Vec<LeanStageDef> = kinds
            .iter()
            .enumerate()
            .map(|(i, kind)| LeanStageDef::skipped(format!("h{i}"), *kind, i, span()))
            .collect();
        let mut out = String::new();
        let outcome = run_into(&printer, &defs, &mut out).unwrap();
        assert_eq!(outcome.definitions, 5);
        assert_eq!(outcome.emitted, 0);
        assert!(outcome.registers.is_empty());
        assert_eq!(outcome.skipped.len(), 5);
        assert!(out.is_empty());
        assert!(outcome.is_empty());
        for (skip, kind) in outcome.skipped.iter().zip(kinds) {
            assert_eq!(skip.kind, kind.as_str());
        }
    }

    #[test]
    fn missing_signature_is_unsupported_exit_1() {
        let printer = LeanPrinter::with_default();
        let mut def = fixture_function(0);
        def.sig = None;
        let mut out = String::new();
        let mut outcome = LeanOutcome::default();
        let err = emit_one(&printer, &def, &mut out, &mut outcome).unwrap_err();
        assert_eq!(err.exit_code, crate::args::EXIT_ERROR);
        assert!(err.message.contains("lean-emit"), "{err}");
        assert!(err.message.contains("file0:4-9"), "{err}");
        assert!(out.is_empty());
    }

    #[test]
    fn missing_body_is_refused_never_sorry() {
        let printer = LeanPrinter::with_default();
        let mut def = fixture_function(0);
        def.body = None;
        let mut out = String::new();
        let mut outcome = LeanOutcome::default();
        let err = emit_one(&printer, &def, &mut out, &mut outcome).unwrap_err();
        assert_eq!(err.exit_code, crate::args::EXIT_ERROR);
        assert!(err.message.contains("no silent sorry"), "{err}");
        assert!(err.message.contains("file0:4-9"), "{err}");
        assert!(out.is_empty());
    }

    #[test]
    fn missing_register_type_is_unsupported_exit_1() {
        let printer = LeanPrinter::with_default();
        let mut def = fixture_register(0);
        def.register_ty = None;
        let mut out = String::new();
        let mut outcome = LeanOutcome::default();
        let err = emit_one(&printer, &def, &mut out, &mut outcome).unwrap_err();
        assert_eq!(err.exit_code, crate::args::EXIT_ERROR);
        assert!(err.message.contains("register `r`"), "{err}");
        assert!(err.message.contains("file0:4-9"), "{err}");
        assert!(out.is_empty());
    }

    #[test]
    fn proof_holes_are_refused_before_writing() {
        let printer = LeanPrinter::with_default();
        for bad in [
            "sorry",
            "  sorry",
            "(sorry)",
            "do sorry",
            "admit",
            "by admit",
            "x + sorry",
        ] {
            assert!(body_has_hole(bad), "{bad:?}");
            let mut def = fixture_function(0);
            def.body = Some(bad.to_owned());
            let mut out = String::new();
            let mut outcome = LeanOutcome::default();
            let err = emit_one(&printer, &def, &mut out, &mut outcome).unwrap_err();
            assert_eq!(err.exit_code, crate::args::EXIT_ERROR, "{bad:?}");
            assert!(err.message.contains("proof hole"), "{bad:?}");
            assert!(out.is_empty(), "{bad:?}");
        }
    }

    #[test]
    fn hole_scan_ignores_strings_comments_and_substrings() {
        assert!(!body_has_hole("mysorry + sorry_x + admittedly"));
        assert!(!body_has_hole("\"sorry\""));
        assert!(!body_has_hole("x -- sorry\n+ y"));
        assert!(!body_has_hole("x /- sorry -/ + y"));
        assert!(!body_has_hole(
            "x /- outer /- sorry -/ still comment -/ + y"
        ));
        assert!(body_has_hole("\"ok\" ++ sorry"));
        assert!(body_has_hole("-- comment\nsorry"));
        // …and clean bodies still emit.
        let printer = LeanPrinter::with_default();
        let mut def = fixture_function(0);
        def.body = Some("\"sorry\"".to_owned());
        let (outcome, out) = render_all(&printer, &[def]).unwrap();
        assert_eq!(outcome.emitted, 1);
        assert!(out.contains("\"sorry\""));
    }

    #[test]
    fn first_hard_error_stops_the_stream() {
        let printer = LeanPrinter::with_default();
        let mut bad = fixture_function(1);
        bad.body = Some("sorry".to_owned());
        let defs = vec![fixture_value(0), bad, fixture_value(2)];
        let mut out = String::new();
        let err = run_into(&printer, &defs, &mut out).unwrap_err();
        assert_eq!(err.exit_code, crate::args::EXIT_ERROR);
        assert!(out.contains("def v : Nat := 42\n"));
        assert!(!out.contains("sorry"));
    }

    #[test]
    fn empty_run_is_vacuous() {
        let printer = LeanPrinter::with_default();
        let (outcome, out) = render_all(&printer, &[]).unwrap();
        assert_eq!(outcome.definitions, 0);
        assert_eq!(outcome.emitted, 0);
        assert!(outcome.is_empty());
        assert!(out.is_empty());
    }
}
