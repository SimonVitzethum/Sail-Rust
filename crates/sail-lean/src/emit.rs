#![forbid(unsafe_code)]
//! Typed-definition emission: elaborated schemes plus real bodies onto Lean.
//!
//! Lane 33. This module turns an elaborated typed definition — its declared
//! [`TypSchm`](sail_ast::TypSchm) lowered through
//! [`typ_from_ast`](crate::conv::typ_from_ast) — plus an already-printed
//! body into a complete Lean `def`. It never invents bodies itself:
//! - `body: None` (untranslatable shape in this lane) is a hard
//!   [`Error::Unsupported`][crate::Error], never a silent `sorry`.
//! - body text containing a proof hole (`sorry` / `admit` as Lean tokens)
//!   is refused with [`Error::Unsupported`][crate::Error] before anything
//!   is written.
//!
//! ## Where bodies come from
//!
//! Body printing itself lives in `sail-lean/src/exps.rs` (Lane 14,
//! `LeanPrinter::print_fun_def` / `render_fun_def` over canonical
//! `sail_ast::Exp` with an `ExpCtx` environment).
//! That module is the merged-tree counterpart of this one: the production
//! driver prints the clause body with `exps.rs` and hands the resulting
//! text to [`emit_def`] / [`render_def`] here together with the
//! [`FunSig`][crate::FunSig] built by [`funsig_from_scheme`]. This module
//! therefore compiles against the Lane-10 printer core only
//! (`lib.rs` + `conv.rs` + `pats.rs`) and stays linkable with or without
//! `exps.rs`; the NoSorry contract holds on both sides because neither
//! side can fabricate a proof hole.
//!
//! ## Kind coverage
//!
//! [`EmitKind`] covers the three value-like declaration forms:
//! functions, `val` values (plain or function-typed) and registers.
//! Registers do not become `def`s here: [`register_entry`] lowers the
//! declared type and returns a `(name, lean_type)` pair for the
//! `sail-lake` register templates (`render_register_inductive` /
//! `render_register_type`), which keep owning the `Register` inductive.
//! Header-only forms (type declarations, constraints, pragmas, …) have no
//! scheme payload and are the caller's to skip — see the insert spec below.
//!
//! ## Effect flags
//!
//! The `(is_monadic, early_return, has_loop)` triple ([`EffectFlags`]) is
//! NOT recoverable from the scheme; the driver must compute it from effect
//! analysis over the clause body (the `exps.rs` predicates
//! `exp_has_effect` / `exp_has_early_return` / `exp_has_loop`, mirroring
//! OCaml `doc_funcl_init`). Guessing purity here would silently select the
//! wrong Lean frame, so there is no defaulting: every constructor takes an
//! explicit [`EffectFlags`].
//!
//! ## MAIN-RS-INSERT-SPEC (for the `lean-emit` wiring owner; `main.rs` is
//! NOT touched by this lane)
//!
//! ```ignore
//! // In run_compiler, after typecheck::run(&frontend) yields Checked:
//! // 1. checked.require_typed_program() still fails until this step exists.
//! // 2. For each def in checked.defs (source order via
//! //    TypedDef::sort_by_position, already sorted by typecheck::run):
//! //    - match def.kind (sail_ty::TypedDefKind):
//! //      Function | Value => {
//! //        let scheme = def.scheme().ok_or(missing_scheme)?; // loud, no sorry
//! //        let it = def.interner().ok_or(missing_interner)?;
//! //        // binder names from the loaded clause patterns (pats::pat_from_ast
//! //        // binders / clause PExp), effect flags from effect analysis:
//! //        let sig = sail_lean::emit::funsig_from_scheme(
//! //          &def.name, emit_kind, scheme, &param_names, flags, span, it)?;
//! //        // body text via exps.rs render_fun_def body part
//! //        // (LeanPrinter::print_exp_top with ExpCtx), then:
//! //        sail_lean::emit::emit_def(&mut out, &printer, &sig, Some(&body_text))?;
//! //      }
//! //      Register => {
//! //        let (ctor, ty) = sail_lean::emit::register_entry(
//! //          &printer, &def.name, scheme, span, it)?;
//! //        register_entries.push((ctor, ty));
//! //      }
//! //      Type | Constraint | Overload | Let | Other => {
//! //        // header-only or out-of-scope form: skip loudly in verbose log,
//! //        // never emit a placeholder def.
//! //      }
//! // 3. Feed register_entries into sail_lake::render_register_* (templates
//! //    unchanged; goldens only on template change).
//! // 4. Every `?` above surfaces Error::Unsupported (with Span) as exit 1.
//! //    No path may write `sorry`/`admit`.
//! ```
//!
//! The elaborated `scripts/CheckLeanNoSorry.lean` gate stays authoritative
//! for proof holes in produced files; the token scan here
//! ([`body_has_hole`]) is emission-time defense in depth.

use std::fmt;

use sail_ast::{Interner, TypKind, TypSchm};

use crate::conv::typ_from_ast;
use crate::{Error, FunSig, LeanPrinter, Span};

/// Which value-like declaration form to emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitKind {
    /// `function f …` / mapping clause: scheme must be a `Typ_fn`.
    Function,
    /// `val` spec + `let` body: plain type or `Typ_fn` (with binders).
    Value,
    /// `register r : typ`: goes to [`register_entry`], never a `def`.
    Register,
}

impl EmitKind {
    /// Short label for error messages.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Value => "value",
            Self::Register => "register",
        }
    }
}

/// The three independent OCaml predicates (`doc_funcl_init`): `not
/// (Effects.function_is_pure …)`, `has_early_return exp`, `has_loop exp`.
/// The return-type prefix and the body frame follow the matrix in the
/// crate docs. No implicit default: callers pass what effect analysis
/// computed over the clause body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EffectFlags {
    /// `not (Effects.function_is_pure …)`: return type gains `SailM`.
    pub is_monadic: bool,
    /// `has_early_return exp`: body frame gains the `ExceptM` layer.
    pub early_return: bool,
    /// `has_loop exp`: pure bodies gain the `Id.run` frame.
    pub has_loop: bool,
}

/// Build a [`FunSig`][crate::FunSig] from a declared scheme.
///
/// - `Function` requires a `Typ_fn` scheme and exactly one binder name per
///   domain type; anything else is a hard [`Error::Unsupported`].
/// - `Value` accepts a `Typ_fn` (same binder rule) or a plain type (no
///   binders allowed).
/// - `Register` is rejected here on purpose: registers are
///   [`register_entry`] pairs for the `sail-lake` templates, never `def`s.
///   (A `Typ_fn`-typed register is likewise rejected there.)
/// - A non-empty quantifier telescope (`forall …`) is rejected: implicit
///   binder lowering is a later lane; dropping constraints silently would
///   change the meaning of the emitted `def`.
/// - Types outside the printer subset ([`typ_from_ast`]) are rejected with
///   the offending scheme span, never defaulted.
pub fn funsig_from_scheme(
    name: &str,
    kind: EmitKind,
    scheme: &TypSchm,
    param_names: &[String],
    effects: EffectFlags,
    span: Span,
    it: &impl Interner,
) -> Result<FunSig, Error> {
    if name.is_empty() {
        return Err(Error::Unsupported {
            msg: format!("anonymous {} is not translatable", kind.as_str()),
            span,
        });
    }
    if kind == EmitKind::Register {
        return Err(Error::Unsupported {
            msg: format!(
                "register `{name}` is not a Lean `def` (route through register_entry for the sail-lake Register inductive)"
            ),
            span,
        });
    }
    if !scheme.quant.is_empty() {
        return Err(Error::Unsupported {
            msg: format!(
                "`{}` carries {} quantifier(s): polymorphic schemes need explicit binder lowering (later lane)",
                kind.as_str(),
                scheme.quant.len()
            ),
            span,
        });
    }
    match &scheme.typ.kind {
        TypKind::Fn { args, ret } => {
            if param_names.len() != args.len() {
                return Err(Error::Unsupported {
                    msg: format!(
                        "{} `{name}` has {} domain type(s) but {} binder name(s): clause binders are required",
                        kind.as_str(),
                        args.len(),
                        param_names.len()
                    ),
                    span: scheme.typ.span,
                });
            }
            let mut lowered = Vec::with_capacity(args.len());
            for (i, a) in args.iter().enumerate() {
                match typ_from_ast(a, it) {
                    Some(t) => lowered.push((param_names[i].clone(), t)),
                    None => {
                        return Err(Error::Unsupported {
                            msg: format!(
                                "{} `{name}` argument {i} type is not translatable",
                                kind.as_str()
                            ),
                            span: a.span,
                        });
                    }
                }
            }
            match typ_from_ast(ret, it) {
                Some(t) => Ok(FunSig {
                    name: name.to_owned(),
                    args: lowered,
                    ret: t,
                    is_monadic: effects.is_monadic,
                    early_return: effects.early_return,
                    has_loop: effects.has_loop,
                    span,
                }),
                None => Err(Error::Unsupported {
                    msg: format!("{} `{name}` return type is not translatable", kind.as_str()),
                    span: ret.span,
                }),
            }
        }
        _ => {
            if kind == EmitKind::Function {
                return Err(Error::Unsupported {
                    msg: format!(
                        "function `{name}` scheme is not a function type (no domain to bind)"
                    ),
                    span: scheme.typ.span,
                });
            }
            if !param_names.is_empty() {
                return Err(Error::Unsupported {
                    msg: format!(
                        "value `{name}` has a plain type but {} binder name(s) were supplied",
                        param_names.len()
                    ),
                    span,
                });
            }
            match typ_from_ast(&scheme.typ, it) {
                Some(t) => Ok(FunSig {
                    name: name.to_owned(),
                    args: Vec::new(),
                    ret: t,
                    is_monadic: effects.is_monadic,
                    early_return: effects.early_return,
                    has_loop: effects.has_loop,
                    span,
                }),
                None => Err(Error::Unsupported {
                    msg: format!("value `{name}` type is not translatable"),
                    span: scheme.typ.span,
                }),
            }
        }
    }
}

/// Lower a register declaration to a `(constructor, lean_type)` pair for
/// the `sail-lake` register templates.
///
/// The type must be a plain translatable type: function types and
/// quantified schemes are hard errors (same policy as
/// [`funsig_from_scheme`]). The name is returned verbatim — constructor
/// spelling stays owned by the templates (goldens unchanged).
pub fn register_entry(
    printer: &LeanPrinter,
    name: &str,
    scheme: &TypSchm,
    span: Span,
    it: &impl Interner,
) -> Result<(String, String), Error> {
    if name.is_empty() {
        return Err(Error::Unsupported {
            msg: "anonymous register is not translatable".to_string(),
            span,
        });
    }
    if !scheme.quant.is_empty() {
        return Err(Error::Unsupported {
            msg: format!(
                "register `{name}` carries {} quantifier(s): polymorphic schemes need explicit binder lowering (later lane)",
                scheme.quant.len()
            ),
            span,
        });
    }
    if matches!(scheme.typ.kind, TypKind::Fn { .. }) {
        return Err(Error::Unsupported {
            msg: format!("register `{name}` has a function type, which is not translatable"),
            span: scheme.typ.span,
        });
    }
    match typ_from_ast(&scheme.typ, it) {
        Some(t) => {
            let mut ty = String::new();
            printer.print_typ(&mut ty, &t)?;
            Ok((name.to_owned(), ty))
        }
        None => Err(Error::Unsupported {
            msg: format!("register `{name}` type is not translatable"),
            span: scheme.typ.span,
        }),
    }
}

/// Whether Lean body text contains a proof hole.
///
/// Token scan over `sorry` / `admit` as standalone Lean identifiers.
/// String literals (`"…"`, Lean `\"` escapes), line comments (`-- …`) and
/// block comments (`/- … -/`, nested) are skipped first, so a Sail string
/// `"sorry"` or a comment mentioning a hole cannot fail a legitimate
/// definition. This is emission-time defense in depth; the authoritative
/// check stays the elaborated `scripts/CheckLeanNoSorry.lean` gate
/// (`hasSorry` over types, values, opaque constants and transitive
/// imports).
#[must_use]
pub fn body_has_hole(text: &str) -> bool {
    for token in lean_code_tokens(text) {
        if token == "sorry" || token == "admit" {
            return true;
        }
    }
    false
}

/// Emit one complete `def`: signature header plus frame plus the
/// caller-printed body.
///
/// - `body: None` models "body not translatable in this lane" and fails
///   with [`Error::Unsupported`] (no silent `sorry`; same rule as
///   [`LeanPrinter::print_fun_stub`][crate::LeanPrinter::print_fun_stub]).
/// - `Some` bodies pass through [`body_has_hole`]: proof holes are refused
///   before anything is written, so `out` is untouched on that path.
/// - The emitted text itself is hole-free by construction (fixed frames
///   plus validated caller text).
pub fn emit_def<W: fmt::Write>(
    out: &mut W,
    printer: &LeanPrinter,
    sig: &FunSig,
    body: Option<&str>,
) -> Result<(), Error> {
    let text = match body {
        Some(b) => b,
        None => {
            return Err(Error::Unsupported {
                msg: format!(
                    "function `{}` body is not translatable (no silent sorry)",
                    sig.name
                ),
                span: sig.span,
            });
        }
    };
    if body_has_hole(text) {
        return Err(Error::Unsupported {
            msg: format!(
                "function `{}` body contains a proof hole (sorry/admit): refusing to emit",
                sig.name
            ),
            span: sig.span,
        });
    }
    printer.print_fun_stub(out, sig, Some(text))
}

/// [`emit_def`] into a fresh `String`.
pub fn render_def(
    printer: &LeanPrinter,
    sig: &FunSig,
    body: Option<&str>,
) -> Result<String, Error> {
    let mut buf = String::new();
    emit_def(&mut buf, printer, sig, body)?;
    Ok(buf)
}

/// Emit definitions in order into one sink (streaming: each `def` renders
/// into a small per-def buffer first, so peak RAM stays O(largest single
/// definition)). Returns the first hard error encountered, if any; defs
/// before it are already written, defs after it are not attempted.
pub fn emit_all<W: fmt::Write>(
    out: &mut W,
    printer: &LeanPrinter,
    defs: &[(FunSig, Option<String>)],
) -> Result<(), Error> {
    for (sig, body) in defs {
        emit_def(out, printer, sig, body.as_deref())?;
    }
    Ok(())
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
        // Line comment: `--` followed by non-`-`? Lean treats `--` as a
        // comment start unconditionally outside strings.
        if c == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            flush!(i);
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment open.
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
        // String literal.
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
        // Character literal `'x'` / escaped: skip so a `'` inside does not
        // glue adjacent code tokens. Only when it looks like a char literal:
        // `'` … exactly one char/escape … `'`.
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
    use sail_ast::{FileId, Name, Node, SimpleInterner, Typ};

    fn span() -> Span {
        Span::new(FileId(0), 0, 1)
    }

    fn id_typ(it: &mut SimpleInterner, name: &str) -> Typ {
        Node::dummy(TypKind::Id(Name::dummy(it.intern(name))))
    }

    fn fn_scheme(it: &mut SimpleInterner, args: &[&str], ret: &str) -> TypSchm {
        let arg_tys: Vec<Typ> = args.iter().map(|a| id_typ(it, a)).collect();
        let ret_ty = id_typ(it, ret);
        TypSchm {
            quant: Vec::new(),
            typ: Node::dummy(TypKind::Fn {
                args: arg_tys,
                ret: Box::new(ret_ty),
            }),
        }
    }

    fn plain_scheme(it: &mut SimpleInterner, name: &str) -> TypSchm {
        TypSchm {
            quant: Vec::new(),
            typ: id_typ(it, name),
        }
    }

    fn pure() -> EffectFlags {
        EffectFlags::default()
    }

    #[test]
    fn function_scheme_builds_typed_signature() {
        let mut it = SimpleInterner::new();
        let scheme = fn_scheme(&mut it, &["int", "bool"], "int");
        let sig = funsig_from_scheme(
            "add",
            EmitKind::Function,
            &scheme,
            &["x".to_string(), "y".to_string()],
            pure(),
            span(),
            &it,
        )
        .unwrap();
        assert_eq!(sig.name, "add");
        assert_eq!(sig.args.len(), 2);
        assert_eq!(sig.ret, crate::Typ::Int);
        assert!(!sig.is_monadic);
    }

    #[test]
    fn value_with_plain_type_needs_no_binders() {
        let mut it = SimpleInterner::new();
        let scheme = plain_scheme(&mut it, "nat");
        let sig =
            funsig_from_scheme("v", EmitKind::Value, &scheme, &[], pure(), span(), &it).unwrap();
        assert!(sig.args.is_empty());
        assert_eq!(sig.ret, crate::Typ::Nat);
        let printer = LeanPrinter::with_default();
        let got = render_def(&printer, &sig, Some("42")).unwrap();
        assert_eq!(got, "def v : Nat := 42\n");
        assert!(!got.contains("sorry"));
        assert!(!got.contains("admit"));
    }

    #[test]
    fn effect_flags_reach_signature_and_frame() {
        let mut it = SimpleInterner::new();
        let scheme = fn_scheme(&mut it, &["int"], "int");
        let flags = EffectFlags {
            is_monadic: true,
            early_return: true,
            has_loop: false,
        };
        let sig = funsig_from_scheme(
            "f",
            EmitKind::Function,
            &scheme,
            &["x".to_string()],
            flags,
            span(),
            &it,
        )
        .unwrap();
        assert!(sig.is_monadic && sig.early_return && !sig.has_loop);
        let printer = LeanPrinter::with_default();
        let got = render_def(&printer, &sig, Some("x")).unwrap();
        assert_eq!(got, "def f (x : Int) : SailM Int := SailME.run do\n  x\n");
    }

    #[test]
    fn register_entry_feeds_lake_templates() {
        let mut it = SimpleInterner::new();
        let scheme = plain_scheme(&mut it, "bool");
        let printer = LeanPrinter::with_default();
        let (ctor, ty) = register_entry(&printer, "my_reg", &scheme, span(), &it).unwrap();
        assert_eq!(ctor, "my_reg");
        assert_eq!(ty, "Bool");
    }

    #[test]
    fn register_rejects_function_types() {
        let mut it = SimpleInterner::new();
        let scheme = fn_scheme(&mut it, &["int"], "int");
        let printer = LeanPrinter::with_default();
        let err = register_entry(&printer, "r", &scheme, span(), &it).unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }));
    }

    #[test]
    fn register_is_not_a_def() {
        let mut it = SimpleInterner::new();
        let scheme = plain_scheme(&mut it, "bool");
        let err = funsig_from_scheme("r", EmitKind::Register, &scheme, &[], pure(), span(), &it)
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }));
        assert!(err.to_string().contains("register_entry"));
    }

    #[test]
    fn binder_arity_mismatch_is_a_hard_error() {
        let mut it = SimpleInterner::new();
        let scheme = fn_scheme(&mut it, &["int", "int"], "int");
        let err = funsig_from_scheme(
            "f",
            EmitKind::Function,
            &scheme,
            &["x".to_string()],
            pure(),
            span(),
            &it,
        )
        .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }));
        assert!(err.to_string().contains("binder"));
    }

    #[test]
    fn function_needs_a_function_scheme() {
        let mut it = SimpleInterner::new();
        let scheme = plain_scheme(&mut it, "int");
        let err = funsig_from_scheme("f", EmitKind::Function, &scheme, &[], pure(), span(), &it)
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }));
    }

    #[test]
    fn polymorphic_schemes_are_refused_not_dropped() {
        use sail_ast::{KOpt, Kind, QuantItem};
        let mut it = SimpleInterner::new();
        let kid = it.intern("n");
        let scheme = TypSchm {
            quant: vec![QuantItem::Id(KOpt {
                kind: Kind::Int,
                kid,
            })],
            typ: id_typ(&mut it, "int"),
        };
        let err = funsig_from_scheme("f", EmitKind::Value, &scheme, &[], pure(), span(), &it)
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }));
        assert!(err.to_string().contains("quantifier"));
        let printer = LeanPrinter::with_default();
        let err = register_entry(&printer, "r", &scheme, span(), &it).unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }));
    }

    #[test]
    fn untranslatable_types_are_hard_errors() {
        let mut it = SimpleInterner::new();
        let scheme = TypSchm {
            quant: Vec::new(),
            typ: Node::dummy(TypKind::Unknown),
        };
        let err = funsig_from_scheme("f", EmitKind::Value, &scheme, &[], pure(), span(), &it)
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }));
    }

    #[test]
    fn anonymous_names_are_refused() {
        let mut it = SimpleInterner::new();
        let scheme = plain_scheme(&mut it, "int");
        let err =
            funsig_from_scheme("", EmitKind::Value, &scheme, &[], pure(), span(), &it).unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }));
        let printer = LeanPrinter::with_default();
        let err = register_entry(&printer, "", &scheme, span(), &it).unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }));
    }

    #[test]
    fn missing_body_is_refused_never_sorry() {
        let mut it = SimpleInterner::new();
        let scheme = plain_scheme(&mut it, "nat");
        let sig =
            funsig_from_scheme("v", EmitKind::Value, &scheme, &[], pure(), span(), &it).unwrap();
        let printer = LeanPrinter::with_default();
        let err = render_def(&printer, &sig, None).unwrap_err();
        assert_eq!(
            err,
            Error::Unsupported {
                msg: "function `v` body is not translatable (no silent sorry)".to_string(),
                span: span(),
            }
        );
        // The error names the policy; no Lean output was fabricated.
        let mut buf = String::new();
        let rc = emit_def(&mut buf, &printer, &sig, None);
        assert!(rc.is_err());
        assert!(buf.is_empty());
    }

    #[test]
    fn proof_holes_in_bodies_are_refused_before_writing() {
        let mut it = SimpleInterner::new();
        let scheme = plain_scheme(&mut it, "nat");
        let sig =
            funsig_from_scheme("v", EmitKind::Value, &scheme, &[], pure(), span(), &it).unwrap();
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
            let mut buf = String::new();
            let err = emit_def(&mut buf, &printer, &sig, Some(bad)).unwrap_err();
            assert!(matches!(err, Error::Unsupported { .. }), "{bad:?}");
            assert!(err.to_string().contains("proof hole"), "{bad:?}");
            assert!(buf.is_empty(), "{bad:?}");
        }
    }

    #[test]
    fn hole_scan_ignores_strings_comments_and_substrings() {
        // Substrings of larger identifiers are not holes.
        assert!(!body_has_hole("mysorry + sorry_x + admittedly"));
        // String literals, line and (nested) block comments are skipped.
        assert!(!body_has_hole("\"sorry\""));
        assert!(!body_has_hole("x -- sorry\n+ y"));
        assert!(!body_has_hole("x /- sorry -/ + y"));
        assert!(!body_has_hole(
            "x /- outer /- sorry -/ still comment -/ + y"
        ));
        assert!(!body_has_hole("\"a\\\"sorry\""));
        // …but real holes beside them still trip the guard.
        assert!(body_has_hole("\"ok\" ++ sorry"));
        assert!(body_has_hole("-- comment\nsorry"));
    }

    #[test]
    fn emitted_output_never_contains_proof_holes() {
        let printer = LeanPrinter::with_default();
        let mut it = SimpleInterner::new();
        let adder = fn_scheme(&mut it, &["int", "int"], "int");
        let sig = funsig_from_scheme(
            "adder",
            EmitKind::Function,
            &adder,
            &["x".to_string(), "y".to_string()],
            pure(),
            span(),
            &it,
        )
        .unwrap();
        let val = plain_scheme(&mut it, "bool");
        let vsig =
            funsig_from_scheme("flag", EmitKind::Value, &val, &[], pure(), span(), &it).unwrap();
        let defs = vec![
            (sig, Some("x + y".to_string())),
            (vsig, Some("true".to_string())),
        ];
        let mut out = String::new();
        emit_all(&mut out, &printer, &defs).unwrap();
        assert!(out.contains("def adder (x : Int) (y : Int) : Int := x + y\n"));
        assert!(out.contains("def flag : Bool := true\n"));
        for hole in ["sorry", "admit", "axiom", "sorryAx"] {
            assert!(!out.contains(hole), "output contains {hole:?}:\n{out}");
        }
    }

    #[test]
    fn emit_all_stops_at_first_hard_error() {
        let printer = LeanPrinter::with_default();
        let mut it = SimpleInterner::new();
        let ok_scheme = plain_scheme(&mut it, "nat");
        let ok = funsig_from_scheme("ok", EmitKind::Value, &ok_scheme, &[], pure(), span(), &it)
            .unwrap();
        let bad_scheme = plain_scheme(&mut it, "nat");
        let bad = funsig_from_scheme(
            "bad",
            EmitKind::Value,
            &bad_scheme,
            &[],
            pure(),
            span(),
            &it,
        )
        .unwrap();
        let defs = vec![
            (ok, Some("1".to_string())),
            (bad, Some("sorry".to_string())),
        ];
        let mut out = String::new();
        let err = emit_all(&mut out, &printer, &defs).unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }));
        // First def already streamed; the hole never reached the sink.
        assert!(out.contains("def ok : Nat := 1\n"));
        assert!(!out.contains("sorry"));
        assert!(!out.contains("def bad"));
    }
}
