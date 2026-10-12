#![forbid(unsafe_code)]
//! FunSig coverage inventory + sound extensions (Lane 69).
//!
//! Reference (main worktree, read-only): `emit.rs::funsig_from_scheme`
//! rejects every non-empty quantifier telescope, every [`TypKind::Unknown`],
//! every [`TypKind::Bidir`] and every anonymous name with
//! [`Error::Unsupported`][crate::Error]. This module answers the Lane-69
//! question — *which ARM signature shapes fail, in ARM-frequency order, and
//! what of it lowers soundly* — without touching `emit.rs` / `conv.rs` /
//! `main.rs` (see `ANDOCK-SPEC` below).
//!
//! ## Inventory: ARMv9.4-a `val` specs (measured 2026-10-12)
//!
//! `sail-arm/arm-v9.4-a`, 12184 `val` lines total.
//!
//! | # | Shape | ARM count | Before | After (this module) |
//! |---|---|---|---|
//! | 1 | Quantified `forall …` (`'d 'datasize 'n …` + numeric/set constraints) | 6070 (~49.8%) | `Unsupported` (quantifier) | **lowered** to [`ExtSig`]: explicit `{k_…}` binders for vars used in the signature, full telescope preserved in a `/-- Type quantifiers: … -/` comment (OCaml `doc_typ_quant_in_comment` parity) |
//! | 2 | Plain monomorphic `Fn` / value (`(bits(4),…) -> unit`, `unit -> bool`, …) | 6114 (~50.2%) | supported | supported (unchanged path) |
//! | 3 | `int('x)` / `bool('x)` domain refinements (parse to `atom` / `atom_bool`) | ~3441 `int('t)`-style + 495 `('b : Bool)` binders | mis-lowered to `(atom …)` / `(atom_bool …)` names | **lowered**: `atom → Int` (`Nat` when locally provably non-negative), `atom_bool → Bool` (OCaml `doc_typ` parity) |
//! | 4 | `-> bits('<var>)` returns (`integer_subrange`, `sub_bits_int`, …) | 180 | `Unsupported` (quantifier) | **lowered** with `{k_… : Int}` binders |
//! | 5 | `('a : Type)` kind vars (`eq_any`, `neq_any`, extern `pure`) | 2 | `Unsupported` (quantifier) | **lowered** with `{k_a : Type}` binders |
//! | 6 | `range(lo,hi)` domain types | 56 in `val` continuations | `(range …)` name | **lowered** to `Nat`/`Int` via non-negativity of `lo` (OCaml parity) |
//! | 7 | `implicit('n)` binders (`Ones`, `Zeros`, `Replicate__1`, …) | 9 sites in `src/` | `Unsupported` | stays [`Error::Unsupported`][crate::Error] (`SigFail::ImplicitArg`): needs body-driven elaboration, OCaml prints `_` for inference — guessing a type would be silent invention |
//! | 8 | `register(…)` types in `val` position | 3 | `(register …)` name | stays `Unsupported` (`SigFail::RegisterRef`): needs the Sail runtime `RegisterRef` supplied by `sail-lake`, not the printer |
//! | 9 | `mapping` / [`TypKind::Bidir`] val specs | 0 in ARMv9.4-a (`map_clauses.sail` holds plain `function`s) | `Unsupported` | stays `Unsupported`, but [`bidir_sigs`] splits a bare `Bidir` into `…_forwards` / `…_backwards` `def`s (OCaml `mapping_regex` parity) |
//! | 10 | `Typ_exist` (`{8,16,…}` set types elaborate to it) | rare in `val` position | `Unsupported` | stays `Unsupported` (`SigFail::Existential`): dropping binders would widen `exists` to `forall` (OCaml does this; we refuse without an explicit opt-in lane) |
//! | 11 | `Typ_unknown` (elaboration placeholders) | 0 in checked defs | `Unsupported` | stays `Unsupported` (`SigFail::Unknown`): no type to emit, any guess is invention |
//! | 12 | Anonymous names | 0 (parser requires names) | `Unsupported` | stays `Unsupported` (`SigForm::Anonymous`) |
//! | 13 | Binder-arity mismatch / stray binders | driver-level, 0 in ARM | `Unsupported` | stays `Unsupported` (precise counts; OCaml's `x_i` fresh-name fallback is deliberately NOT mirrored — it would mask driver bugs) |
//! | 14 | Nested `Fn` (higher-order args), `If`/`Id`/`App` sizes | ~0 in `val` specs (`foreach#` combinators live in bodies, another lane) | `Unsupported` | stays `Unsupported` (`SigFail::NestedFn` / `SigFail::SizeExpr`) |
//!
//! Registers (1392 `register` defs), `overload`s (41) and `scattered`
//! groups (21) are intentionally NOT `def`s: registers feed the
//! `sail-lake` `Register` inductive, overloads/header-only forms are the
//! caller's to skip (same insert-spec policy as `emit.rs`).
//!
//! ## Soundness policy for the new lowerings
//!
//! * `atom(n)` is a Sail integer: Lean `Int` covers every integer, hence
//!   always sound. `Nat` is emitted only when locally proven (`Constant c
//!   >= 0`, or the quantified kid carries `0 <= 'x` / `'x == c>=0` /
//!   all-nonneg-`in {…}` evidence in its own telescope).
//! * `atom_bool(…)` is a Sail boolean: `Bool`, unconditional (OCaml parity).
//! * `range(lo,hi)` is a Sail integer interval: `Nat` iff `lo` is a
//!   non-negative constant, else `Int` (OCaml `provably_nneg low` parity,
//!   minus the solver-backed general case — see docking spec).
//! * Quantifier constraints (`==`, `in {…}`, `>=`, `is_VL(…)`, … — 8376
//!   `==`, 2890 `in {`, 484 `>=` occurrences in ARM) are DROPPED from the
//!   `def` and preserved verbatim-ish in the quantifier comment. This is a
//!   deliberate widening with exact OCaml precedent (`doc_funcl_init` keeps
//!   only `doc_typ_quant_in_comment`; "Use auto-implicits for type
//!   quantifiers"). It is loud, never silent: the comment names every
//!   dropped constraint.
//! * One deliberate deviation from OCaml: OCaml relies on Lean
//!   `autoImplicit` for kind vars and renames a captured `atom('x)` binder
//!   onto the kid (`captured_typ_var`). We emit explicit `{k_x : …}`
//!   binders instead, reusing only public printer API. A kind var used in
//!   the signature is therefore bound but, absent a rename environment in
//!   `lib.rs`, NOT identified with a same-named value binder: `def f
//!   {k_m : Int} (m : Int) : (BitVec k_m)`. A body returning `BitVec m`
//!   fails Lean elaboration loudly at `lake build` time; the capture
//!   optimisation is specified for the wiring owner (below).
//!
//! ## ANDOCK-SPEC (owners of `emit.rs` / `conv.rs` / `main.rs`; this lane
//! does NOT touch those files)
//!
//! 1. `conv.rs`: add three arms to `typ_from_ast` — `atom`, `atom_bool`,
//!    `range` — by delegating to [`typ_from_ast_ext`] here (or moving it
//!    over verbatim). `implicit` and `register` applications must map to
//!    `None` (this module fences them against the `Named` catch-all;
//!    without the fence they mis-lower to same-named Lean applications).
//!    Bool-typed type arguments stay dropped (existing `Only_relevant`
//!    rule, untouched).
//! 2. `emit.rs::funsig_from_scheme`: replace the blanket quantifier
//!    rejection with a call to [`ext_sig_from_scheme`] for the quantified
//!    case; keep every other rejection bit-identical. Unify the local
//!    `EffectFlags` with [`EffectFlags`] here (field names match
//!    deliberately).
//! 3. `main.rs` driver: for `Bidir`-typed mappings emit the
//!    [`bidir_sigs`] pair; for `ImplicitArg` / `Existential` / `Unknown`
//!    failures surface [`Error::Unsupported`] as exit 1 (no `sorry`).
//! 4. Later lane (needs a constraint environment + rename support in
//!    `lib.rs`): solver-backed `provably_nneg` for `atom`/`range` (replace
//!    [`kid_is_nonneg`]), and the `captured_typ_var` optimisation
//!    (identify `{k_x}` with the `atom('x)` value binder) to remove the
//!    disconnected-binder wart documented above.
//! 5. Latent `lib.rs` wart found while testing (pre-existing, affects every
//!    compound size, not just this lane): `print_typ` does not
//!    parenthesise a top-level `Sum`/`Minus` inside `(BitVec …)` /
//!    `(Vector …)`, so `bits('hi - 'lo + 1)` prints
//!    `(BitVec k_hi - k_lo + 1)`, which Lean parses as
//!    `(BitVec k_hi) - k_lo + 1` and rejects loudly at elaboration. Fix in
//!    `print_typ`/`print_nexp` (parenthesise non-atomic sizes in type
//!    position), with golden updates.
//!
//! The elaborated `scripts/CheckLeanNoSorry.lean` gate stays authoritative
//! for proof holes; this module never emits `sorry`/`admit` (tested).

use std::collections::BTreeSet;
use std::fmt;

use sail_ast::{
    Constraint, ConstraintKind, Interner, KOpt, Kind, NExp, NExpKind, QuantItem, Typ as AstTyp,
    TypArgKind, TypKind, TypSchm,
};

use crate::{conv::typ_from_ast, Error, FunSig, LeanPrinter, Nexp, Span, Typ, TypArg};

/// The three independent OCaml predicates (`doc_funcl_init`).
/// Field-for-field compatible with `emit::EffectFlags` in the main
/// worktree so the wiring owner can unify them (see ANDOCK-SPEC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EffectFlags {
    /// `not (Effects.function_is_pure …)`: return type gains `SailM`.
    pub is_monadic: bool,
    /// `has_early_return exp`: body frame gains the `ExceptM` layer.
    pub early_return: bool,
    /// `has_loop exp`: pure bodies gain the `Id.run` frame.
    pub has_loop: bool,
}

/// Which value-like declaration form to lower.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigKind {
    /// `function f …`: scheme must be a `Typ_fn`.
    Function,
    /// `val` spec + body: plain type or `Typ_fn` (with binders).
    Value,
    /// `register r : typ`: never a `def`; precise routing error.
    Register,
}

impl SigKind {
    /// Short label for error messages (matches `emit::EmitKind`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Value => "value",
            Self::Register => "register",
        }
    }
}

/// Lean type of an explicit kind-var binder (OCaml `doc_kind` parity,
/// minus the solver: `Int` unless locally proven non-negative).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImplicitKind {
    Nat,
    Int,
    Bool,
    Type,
}

impl ImplicitKind {
    /// Lean spelling.
    #[must_use]
    pub fn lean_name(self) -> &'static str {
        match self {
            Self::Nat => "Nat",
            Self::Int => "Int",
            Self::Bool => "Bool",
            Self::Type => "Type",
        }
    }
}

/// One explicit kind-var binder `{k_x : T}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImplicitParam {
    /// Raw kid text as resolved through the interner (e.g. `'n` or `n`);
    /// printed via [`crate::print_kid_name`] (`k_`-prefix rule).
    pub kid: String,
    pub kind: ImplicitKind,
}

/// A lowered signature: the plain [`FunSig`] plus the explicit kind-var
/// binders and the preserved quantifier comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtSig {
    pub sig: FunSig,
    /// Binders for kind vars actually used in the lowered signature
    /// types (relevance filter à la OCaml `doc_typ_quant_relevant`).
    pub implicits: Vec<ImplicitParam>,
    /// `/-- Type quantifiers: … -/` (OCaml `doc_typ_quant_in_comment`
    /// parity); empty when the scheme is monomorphic.
    pub quant_comment: String,
}

/// Precise failure class for a single type that has no printer
/// counterpart. Variants double as stable inventory labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SigFail {
    /// `Typ_unknown`: elaboration placeholder, nothing to emit.
    Unknown,
    /// `Typ_bidir` in value position: needs the forwards/backwards split.
    Bidir,
    /// `Typ_exist`: dropping binders would widen `exists` to `forall`.
    Existential,
    /// `Typ_fn` nested inside an argument or return type (higher-order).
    NestedFn,
    /// `implicit('n)`: needs body-driven elaboration (OCaml `_`).
    ImplicitArg,
    /// `register(…)` type application: needs the Sail runtime.
    RegisterRef,
    /// `Nexp` `If` / `Id` / `App`: no printer counterpart (Lane 25
    /// covers div/mod/abs via extern calls, not sizes).
    SizeExpr(String),
    /// Any other application the subset cannot spell (includes arity
    /// outliers such as `bits()` / `bits(a, b)`).
    BadApp(String),
}

impl SigFail {
    /// Stable label for inventory tables and logs.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Bidir => "Bidir",
            Self::Existential => "Existential",
            Self::NestedFn => "NestedFn",
            Self::ImplicitArg => "ImplicitArg",
            Self::RegisterRef => "RegisterRef",
            Self::SizeExpr(_) => "SizeExpr",
            Self::BadApp(_) => "BadApp",
        }
    }
}

/// Inventory label for one declaration scheme (first-match priority, see
/// module docs). `Quantified` means "needs this lane's lowering";
/// `TypeFailure` means "still `Unsupported` after this lane".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SigForm {
    /// Monomorphic, fully translatable (today's coverage).
    PlainMono,
    /// Non-empty quantifier telescope (vars + constraint count).
    Quantified { vars: usize, constraints: usize },
    /// `Typ_bidir` at top level.
    Bidir,
    /// `Typ_exist` at top level.
    Existential,
    /// `Typ_unknown` at top level.
    Unknown,
    /// `Typ_fn` domain count != binder count.
    ArityMismatch { domains: usize, binders: usize },
    /// Plain-typed `Function` scheme (no domain to bind).
    NotAFunction,
    /// Plain type with stray binder names.
    StrayBinders { binders: usize },
    /// Register routed here instead of the register lowering.
    RegisterAsDef,
    /// Anonymous declaration name.
    Anonymous,
    /// A nested type failure (with location-agnostic detail).
    TypeFailure(SigFail),
}

impl SigForm {
    /// Stable label for inventory tables and logs.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::PlainMono => "PlainMono",
            Self::Quantified { .. } => "Quantified",
            Self::Bidir => "Bidir",
            Self::Existential => "Existential",
            Self::Unknown => "Unknown",
            Self::ArityMismatch { .. } => "ArityMismatch",
            Self::NotAFunction => "NotAFunction",
            Self::StrayBinders { .. } => "StrayBinders",
            Self::RegisterAsDef => "RegisterAsDef",
            Self::Anonymous => "Anonymous",
            Self::TypeFailure(f) => f.code(),
        }
    }
}

/// Classify one declaration scheme without lowering it.
pub fn classify_scheme(
    kind: SigKind,
    scheme: &TypSchm,
    param_names: &[String],
    it: &impl Interner,
) -> SigForm {
    if kind == SigKind::Register {
        return SigForm::RegisterAsDef;
    }
    match &scheme.typ.kind {
        TypKind::Fn { args, .. } => {
            if param_names.len() != args.len() {
                return SigForm::ArityMismatch {
                    domains: args.len(),
                    binders: param_names.len(),
                };
            }
            for a in args {
                if let Some(f) = first_type_failure(a, it) {
                    return SigForm::TypeFailure(f);
                }
            }
            let ret = match &scheme.typ.kind {
                TypKind::Fn { ret, .. } => ret,
                _ => unreachable!("matched Fn above"),
            };
            if let Some(f) = first_type_failure(ret, it) {
                return SigForm::TypeFailure(f);
            }
            quantified_or_plain(scheme)
        }
        TypKind::Bidir { .. } => SigForm::Bidir,
        TypKind::Exist { .. } => SigForm::Existential,
        TypKind::Unknown => SigForm::Unknown,
        _ => {
            if kind == SigKind::Function {
                return SigForm::NotAFunction;
            }
            if !param_names.is_empty() {
                return SigForm::StrayBinders {
                    binders: param_names.len(),
                };
            }
            if let Some(f) = first_type_failure(&scheme.typ, it) {
                return SigForm::TypeFailure(f);
            }
            quantified_or_plain(scheme)
        }
    }
}

fn quantified_or_plain(scheme: &TypSchm) -> SigForm {
    if scheme.quant.is_empty() {
        SigForm::PlainMono
    } else {
        let mut vars = 0;
        let mut constraints = 0;
        for q in &scheme.quant {
            match q {
                QuantItem::Id(_) => vars += 1,
                QuantItem::Constraint(_) => constraints += 1,
            }
        }
        SigForm::Quantified { vars, constraints }
    }
}

/// First nested type failure in source order, if any.
fn first_type_failure(t: &AstTyp, it: &impl Interner) -> Option<SigFail> {
    match &t.kind {
        TypKind::Unknown => Some(SigFail::Unknown),
        TypKind::Bidir { .. } => Some(SigFail::Bidir),
        TypKind::Exist { .. } => Some(SigFail::Existential),
        TypKind::Fn { .. } => Some(SigFail::NestedFn),
        TypKind::Var(_) => None,
        TypKind::Tuple(ts) => ts.iter().find_map(|e| first_type_failure(e, it)),
        TypKind::Id(_) => None,
        TypKind::App { name, args } => {
            let text = it.resolve(name.symbol).unwrap_or("<unresolvable>");
            if text == "implicit" {
                return Some(SigFail::ImplicitArg);
            }
            if text == "register" {
                return Some(SigFail::RegisterRef);
            }
            if (text == "atom" || text == "atom_bool" || text == "range")
                && app_is_supported(text, args, it)
            {
                return None;
            }
            // Recurse: report the nested cause when there is one (e.g. an
            // `Unknown` inside `option(…)`), else the application itself.
            for a in args {
                match &a.kind {
                    TypArgKind::Typ(e) => {
                        if let Some(f) = first_type_failure(e, it) {
                            return Some(f);
                        }
                    }
                    TypArgKind::NExp(n) => {
                        if let Some(detail) = first_nexp_failure(n, it) {
                            return Some(SigFail::SizeExpr(detail));
                        }
                    }
                    TypArgKind::Bool(_) => {}
                }
            }
            // `bits(n)` / `bitvector(n)` / `vector(n, t)` / plain `Id`-like
            // applications lower via `conv`; anything left is genuinely bad.
            if typ_from_ast(t, it).is_some() {
                None
            } else {
                Some(SigFail::BadApp(text.to_owned()))
            }
        }
    }
}

/// Whether an `atom` / `atom_bool` / `range` application has a supported
/// shape (payload lowering may still fail on sizes — reported separately).
fn app_is_supported(name: &str, args: &[sail_ast::TypArg], it: &impl Interner) -> bool {
    match name {
        "atom_bool" => true,
        "atom" => {
            args.len() == 1 && matches!(args[0].kind, TypArgKind::NExp(_)) && {
                // Sizes still need a printer counterpart.
                match &args[0].kind {
                    TypArgKind::NExp(n) => first_nexp_failure(n, it).is_none(),
                    _ => false,
                }
            }
        }
        "range" => {
            args.len() == 2
                && matches!(
                    (&args[0].kind, &args[1].kind),
                    (TypArgKind::NExp(_), TypArgKind::NExp(_))
                )
                && {
                    match (&args[0].kind, &args[1].kind) {
                        (TypArgKind::NExp(lo), TypArgKind::NExp(hi)) => {
                            first_nexp_failure(lo, it).is_none()
                                && first_nexp_failure(hi, it).is_none()
                        }
                        _ => false,
                    }
                }
        }
        _ => false,
    }
}

/// First `NExp` shape without a printer counterpart, if any.
fn first_nexp_failure(n: &NExp, it: &impl Interner) -> Option<String> {
    match &n.kind {
        NExpKind::If { .. } => Some("Nexp_if".to_string()),
        NExpKind::Id(name) => Some(format!(
            "Nexp_id({})",
            it.resolve(name.symbol).unwrap_or("<unresolvable>")
        )),
        NExpKind::App { func, .. } => Some(format!(
            "Nexp_app({})",
            it.resolve(func.symbol).unwrap_or("<unresolvable>")
        )),
        NExpKind::Constant(_) | NExpKind::Var(_) => None,
        NExpKind::Sum(a, b) | NExpKind::Minus(a, b) | NExpKind::Times(a, b) => {
            first_nexp_failure(a, it).or_else(|| first_nexp_failure(b, it))
        }
        NExpKind::Exp(a) | NExpKind::Neg(a) => first_nexp_failure(a, it),
    }
}

/// Lower a canonical type onto the printer subset, extended with the
/// sound `atom` / `atom_bool` / `range` arms (OCaml `doc_typ` parity).
///
/// `nonneg` holds the raw kid texts proven `>= 0` (see
/// [`nonneg_kids`]); every other integer-like payload maps to `Int`,
/// which covers all Sail integers and is therefore always sound.
pub fn typ_from_ast_ext(t: &AstTyp, it: &impl Interner, nonneg: &BTreeSet<String>) -> Option<Typ> {
    if let TypKind::App { name, args } = &t.kind {
        let text = it.resolve(name.symbol)?;
        // Fenced against the `conv` catch-all (`Named`): these two must
        // never lower to a same-named Lean application.
        if text == "implicit" || text == "register" {
            return None;
        }
        if text == "atom_bool" {
            return Some(Typ::Bool);
        }
        if text == "atom" {
            if let [arg] = args.as_slice() {
                if let TypArgKind::NExp(n) = &arg.kind {
                    return Some(if nexp_is_nonneg(n, it, nonneg) {
                        Typ::Nat
                    } else {
                        Typ::Int
                    });
                }
            }
            return None;
        }
        if text == "range" {
            if let [lo, _hi] = args.as_slice() {
                if let TypArgKind::NExp(lo) = &lo.kind {
                    // Mirror OCaml: only the lower bound decides.
                    return Some(if nexp_is_nonneg(lo, it, nonneg) {
                        Typ::Nat
                    } else {
                        Typ::Int
                    });
                }
            }
            return None;
        }
    }
    typ_from_ast(t, it)
}

/// Local non-negativity: constants `>= 0`, or variables with telescope
/// evidence (see [`nonneg_kids`]). Anything else is treated as possibly
/// negative — conservative and always sound.
fn nexp_is_nonneg(n: &NExp, it: &impl Interner, nonneg: &BTreeSet<String>) -> bool {
    match &n.kind {
        NExpKind::Constant(s) => s.parse::<i128>().is_ok_and(|c| c >= 0),
        NExpKind::Var(sym) => it.resolve(*sym).is_some_and(|text| nonneg.contains(text)),
        _ => false,
    }
}

/// Kids with `>= 0` evidence in the scheme telescope: `0 <= 'x` / `'x >=
/// 0` (incl. strict variants over integers), `'x == c >= 0`, and `'x in
/// {…}` with all members non-negative. Conjunctions are entered;
/// disjunctions, negations and function applications (`is_VL(…)`, …)
/// yield no evidence (conservative).
pub fn nonneg_kids(scheme: &TypSchm, it: &impl Interner) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for q in &scheme.quant {
        if let QuantItem::Constraint(c) = q {
            collect_nonneg(c, it, &mut out);
        }
    }
    out
}

fn collect_nonneg(c: &Constraint, it: &impl Interner, out: &mut BTreeSet<String>) {
    match &c.kind {
        ConstraintKind::And(a, b) => {
            collect_nonneg(a, it, out);
            collect_nonneg(b, it, out);
        }
        ConstraintKind::Ge(a, b) => {
            // `'x >= c` with `c >= 0`.
            if let (Some(kid), Some(lo)) = (nexp_var_text(a, it), nexp_const(b)) {
                if lo >= 0 {
                    out.insert(kid);
                }
            }
        }
        ConstraintKind::Le(a, b) => {
            // `c <= 'x` with `c >= 0`.
            if let (Some(lo), Some(kid)) = (nexp_const(a), nexp_var_text(b, it)) {
                if lo >= 0 {
                    out.insert(kid);
                }
            }
        }
        ConstraintKind::Gt(a, b) => {
            // `'x > c` with `c >= -1` (integers: `x > -1 ⟺ x >= 0`).
            if let (Some(kid), Some(lo)) = (nexp_var_text(a, it), nexp_const(b)) {
                if lo >= -1 {
                    out.insert(kid);
                }
            }
        }
        ConstraintKind::Lt(a, b) => {
            // `c < 'x` with `c >= 0` (integers: `0 < x ⟹ x >= 0`).
            if let (Some(lo), Some(kid)) = (nexp_const(a), nexp_var_text(b, it)) {
                if lo >= 0 {
                    out.insert(kid);
                }
            }
        }
        ConstraintKind::Equal(a, b) => {
            // `'x == c` / `c == 'x` with `c >= 0` (`Equal` carries
            // `TypArg`s; only the `NExp` reading counts here).
            if let (Some(kid), Some(v)) = (typarg_var_text(a, it), typarg_const(b)) {
                if v >= 0 {
                    out.insert(kid);
                }
            } else if let (Some(v), Some(kid)) = (typarg_const(a), typarg_var_text(b, it)) {
                if v >= 0 {
                    out.insert(kid);
                }
            }
        }
        ConstraintKind::Set { nexp, members } => {
            if let Some(kid) = nexp_var_text(nexp, it) {
                if !members.is_empty()
                    && members
                        .iter()
                        .all(|m| m.parse::<i128>().is_ok_and(|v| v >= 0))
                {
                    out.insert(kid);
                }
            }
        }
        _ => {}
    }
}

fn nexp_var_text(n: &NExp, it: &impl Interner) -> Option<String> {
    match &n.kind {
        NExpKind::Var(sym) => it.resolve(*sym).map(str::to_owned),
        _ => None,
    }
}

/// `NExp` reading of an equality-side type argument, if any.
fn typarg_nexp(a: &sail_ast::TypArg) -> Option<&NExp> {
    match &a.kind {
        TypArgKind::NExp(n) => Some(n),
        _ => None,
    }
}

fn typarg_var_text(a: &sail_ast::TypArg, it: &impl Interner) -> Option<String> {
    nexp_var_text(typarg_nexp(a)?, it)
}

fn typarg_const(a: &sail_ast::TypArg) -> Option<i128> {
    nexp_const(typarg_nexp(a)?)
}

fn nexp_const(n: &NExp) -> Option<i128> {
    match &n.kind {
        NExpKind::Constant(s) => s.parse::<i128>().ok(),
        _ => None,
    }
}

/// Lower a declared scheme to an [`ExtSig`]: the [`FunSig`] plus explicit
/// kind-var binders for every quantified variable used in the signature
/// types, plus the preserved quantifier comment.
///
/// Policy mirrors `emit::funsig_from_scheme` for every still-unsupported
/// shape (same message wording where the class overlaps), except the
/// quantifier telescope itself, which is lowered here instead of refused.
#[allow(clippy::too_many_arguments)]
pub fn ext_sig_from_scheme(
    name: &str,
    kind: SigKind,
    scheme: &TypSchm,
    param_names: &[String],
    effects: EffectFlags,
    span: Span,
    it: &impl Interner,
) -> Result<ExtSig, Error> {
    if name.is_empty() {
        return Err(Error::Unsupported {
            msg: format!("anonymous {} is not translatable", kind.as_str()),
            span,
        });
    }
    if kind == SigKind::Register {
        return Err(Error::Unsupported {
            msg: format!(
                "register `{name}` is not a Lean `def` (route through the register lowering for the sail-lake Register inductive)"
            ),
            span,
        });
    }
    let nonneg = nonneg_kids(scheme, it);
    let quant_comment = render_quant_comment(scheme, it);
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
                match typ_from_ast_ext(a, it, &nonneg) {
                    Some(t) => lowered.push((param_names[i].clone(), t)),
                    None => return Err(type_failure(name, kind, i, a, true, it)),
                }
            }
            match typ_from_ast_ext(ret, it, &nonneg) {
                Some(t) => {
                    let sig = FunSig {
                        name: name.to_owned(),
                        args: lowered,
                        ret: t,
                        is_monadic: effects.is_monadic,
                        early_return: effects.early_return,
                        has_loop: effects.has_loop,
                        span,
                    };
                    Ok(finish(sig, &scheme.quant, it, quant_comment, &nonneg))
                }
                None => Err(type_failure(name, kind, 0, ret, false, it)),
            }
        }
        TypKind::Bidir { .. } => Err(Error::Unsupported {
            msg: format!(
                "mapping `{name}` has a bidirectional type: emit forwards/backwards `def`s via bidir_sigs, never a single `def`"
            ),
            span: scheme.typ.span,
        }),
        TypKind::Exist { .. } => Err(Error::Unsupported {
            msg: format!(
                "mapping `{name}` exposes an existential type: dropping binders would turn exists into forall (later lane)"
            ),
            span: scheme.typ.span,
        }),
        TypKind::Unknown => Err(Error::Unsupported {
            msg: format!(
                "{} `{name}` type is Unknown (elaboration placeholder): no Lean type to emit",
                kind.as_str()
            ),
            span: scheme.typ.span,
        }),
        _ => {
            if kind == SigKind::Function {
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
            match typ_from_ast_ext(&scheme.typ, it, &nonneg) {
                Some(t) => {
                    let sig = FunSig {
                        name: name.to_owned(),
                        args: Vec::new(),
                        ret: t,
                        is_monadic: effects.is_monadic,
                        early_return: effects.early_return,
                        has_loop: effects.has_loop,
                        span,
                    };
                    Ok(finish(sig, &scheme.quant, it, quant_comment, &nonneg))
                }
                None => Err(type_failure(name, kind, 0, &scheme.typ, false, it)),
            }
        }
    }
}

fn finish(
    sig: FunSig,
    quant: &[QuantItem],
    it: &impl Interner,
    quant_comment: String,
    nonneg: &BTreeSet<String>,
) -> ExtSig {
    let used = used_kids(&sig.args, &sig.ret);
    let implicits = quant
        .iter()
        .filter_map(|q| match q {
            QuantItem::Id(KOpt { kind, kid }) => {
                let text = it.resolve(*kid)?.to_owned();
                if !used.contains(&text) {
                    return None;
                }
                let lean = match kind {
                    Kind::Bool => ImplicitKind::Bool,
                    Kind::Type => ImplicitKind::Type,
                    Kind::Int => {
                        if nonneg.contains(&text) {
                            ImplicitKind::Nat
                        } else {
                            ImplicitKind::Int
                        }
                    }
                };
                Some(ImplicitParam {
                    kid: text,
                    kind: lean,
                })
            }
            QuantItem::Constraint(_) => None,
        })
        .collect();
    ExtSig {
        sig,
        implicits,
        quant_comment,
    }
}

/// Kind-var texts surviving in the lowered signature (relevance filter).
fn used_kids(args: &[(String, Typ)], ret: &Typ) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (_, t) in args {
        collect_typ_vars(t, &mut out);
    }
    collect_typ_vars(ret, &mut out);
    out
}

fn collect_typ_vars(t: &Typ, out: &mut BTreeSet<String>) {
    match t {
        Typ::Var(v) => {
            out.insert(v.clone());
        }
        Typ::BitVec(n) => collect_nexp_vars(n, out),
        Typ::Vector(elem, n) => {
            collect_typ_vars(elem, out);
            collect_nexp_vars(n, out);
        }
        Typ::Option(t) | Typ::List(t) => collect_typ_vars(t, out),
        Typ::Result(a, b) => {
            collect_typ_vars(a, out);
            collect_typ_vars(b, out);
        }
        Typ::Tuple(ts) => {
            for t in ts {
                collect_typ_vars(t, out);
            }
        }
        Typ::Named { args, .. } => {
            for a in args {
                match a {
                    TypArg::Typ(t) => collect_typ_vars(t, out),
                    TypArg::Nexp(n) => collect_nexp_vars(n, out),
                }
            }
        }
        Typ::Unit | Typ::Int | Typ::Nat | Typ::Bool | Typ::String | Typ::Bit(_) => {}
    }
}

fn collect_nexp_vars(n: &Nexp, out: &mut BTreeSet<String>) {
    match n {
        Nexp::Var(v) => {
            out.insert(v.clone());
        }
        Nexp::Const(_) => {}
        Nexp::Sum(a, b) | Nexp::Minus(a, b) | Nexp::Times(a, b) => {
            collect_nexp_vars(a, out);
            collect_nexp_vars(b, out);
        }
        Nexp::Neg(a) | Nexp::Pow2(a) => collect_nexp_vars(a, out),
    }
}

/// Precise `Unsupported` for an untranslatable argument or return type.
fn type_failure(
    name: &str,
    kind: SigKind,
    i: usize,
    t: &AstTyp,
    is_arg: bool,
    it: &impl Interner,
) -> Error {
    let where_span = t.span;
    let fail = first_type_failure(t, it).unwrap_or_else(|| match &t.kind {
        TypKind::App { name, .. } => SigFail::BadApp(
            it.resolve(name.symbol)
                .unwrap_or("<unresolvable>")
                .to_owned(),
        ),
        _ => SigFail::BadApp("<unknown>".to_string()),
    });
    let what = if is_arg {
        format!("argument {i} type")
    } else {
        "return type".to_string()
    };
    let detail = match &fail {
        SigFail::Unknown => {
            "is Unknown (elaboration placeholder): no Lean type to emit".to_string()
        }
        SigFail::Bidir => {
            "is bidirectional: mappings need forwards/backwards `def`s via bidir_sigs".to_string()
        }
        SigFail::Existential => {
            "is existential: dropping binders would turn exists into forall (later lane)"
                .to_string()
        }
        SigFail::NestedFn => {
            "is higher-order: the printer Typ has no function-type former".to_string()
        }
        SigFail::ImplicitArg => {
            "uses implicit('…'): implicit binders need body-driven elaboration (later lane)"
                .to_string()
        }
        SigFail::RegisterRef => {
            "uses register(…): needs the Sail runtime RegisterRef from sail-lake".to_string()
        }
        SigFail::SizeExpr(d) => format!("has an untranslatable size expression ({d})"),
        SigFail::BadApp(app) => format!("type application `{app}` is not translatable"),
    };
    Error::Unsupported {
        msg: format!("{} `{name}` {what} {detail}", kind.as_str()),
        span: where_span,
    }
}

/// Split a `Bidir` mapping type (`lhs <-> rhs`) into its
/// forwards/backwards `def` signatures (`{name}_forwards : lhs -> rhs`,
/// `{name}_backwards : rhs -> lhs`).
///
/// Mapping sides are plain types, not `Typ_fn`: each direction takes one
/// input of its source type and returns the target type. Binder lists
/// with exactly one name bind the whole side; a one-level tuple side may
/// instead be untupled with one binder per element (minimal OCaml
/// `untuple_args_pat` parity). Anything else is a precise `Unsupported`
/// — the driver must supply clause-pattern binders.
///
/// Payloads lower via [`typ_from_ast_ext`] with an empty non-negativity
/// set: quantified mappings stay `Unsupported` until the
/// constraint-environment lane lands.
#[allow(clippy::too_many_arguments)] // symmetric pair API: mirrors the driver call shape.
pub fn bidir_sigs(
    name: &str,
    lhs: &AstTyp,
    rhs: &AstTyp,
    fwd_binders: &[String],
    bwd_binders: &[String],
    effects: EffectFlags,
    span: Span,
    it: &impl Interner,
) -> Result<(FunSig, FunSig), Error> {
    if name.is_empty() {
        return Err(Error::Unsupported {
            msg: "anonymous mapping is not translatable".to_string(),
            span,
        });
    }
    let empty = BTreeSet::new();
    let one = |side: &str,
               from: &AstTyp,
               to: &AstTyp,
               binders: &[String]|
     -> Result<FunSig, Error> {
        if matches!(from.kind, TypKind::Fn { .. }) || matches!(to.kind, TypKind::Fn { .. }) {
            return Err(Error::Unsupported {
                msg: format!("mapping `{name}` {side} side is higher-order: not translatable"),
                span: from.span,
            });
        }
        // One binder binds the whole side; a tuple side may be untupled.
        let domains: Vec<&AstTyp> = match &from.kind {
            TypKind::Tuple(ts) if binders.len() == ts.len() && !ts.is_empty() => {
                ts.iter().collect()
            }
            _ => vec![from],
        };
        if binders.len() != domains.len() {
            return Err(Error::Unsupported {
                msg: format!(
                    "mapping `{name}` {side} side has {} domain type(s) but {} binder name(s): clause binders are required",
                    domains.len(),
                    binders.len()
                ),
                span: from.span,
            });
        }
        let mut lowered = Vec::with_capacity(domains.len());
        for (i, a) in domains.iter().enumerate() {
            match typ_from_ast_ext(a, it, &empty) {
                Some(t) => lowered.push((binders[i].clone(), t)),
                None => {
                    return Err(type_failure(
                        &format!("{name}_{side}"),
                        SigKind::Value,
                        i,
                        a,
                        true,
                        it,
                    ));
                }
            }
        }
        match typ_from_ast_ext(to, it, &empty) {
            Some(ret) => Ok(FunSig {
                name: format!("{name}_{side}"),
                args: lowered,
                ret,
                is_monadic: effects.is_monadic,
                early_return: effects.early_return,
                has_loop: effects.has_loop,
                span,
            }),
            None => Err(type_failure(
                &format!("{name}_{side}"),
                SigKind::Value,
                0,
                to,
                false,
                it,
            )),
        }
    };
    // Forwards: lhs -> rhs. Backwards: rhs -> lhs.
    let fwd = one("forwards", lhs, rhs, fwd_binders)?;
    let bwd = one("backwards", rhs, lhs, bwd_binders)?;
    Ok((fwd, bwd))
}

/// `/-- Type quantifiers: k_x : Int, … -/` preserving the full telescope
/// (OCaml `doc_typ_quant_in_comment` parity). Empty for monomorphic
/// schemes.
fn render_quant_comment(scheme: &TypSchm, it: &impl Interner) -> String {
    if scheme.quant.is_empty() {
        return String::new();
    }
    let nonneg = nonneg_kids(scheme, it);
    let mut parts = Vec::new();
    for q in &scheme.quant {
        match q {
            QuantItem::Id(KOpt { kind, kid }) => {
                let raw = it.resolve(*kid).unwrap_or("<unresolvable>");
                let kind_name = match kind {
                    Kind::Bool => "Bool",
                    Kind::Type => "Type",
                    Kind::Int => {
                        if nonneg.contains(raw) {
                            "Nat"
                        } else {
                            "Int"
                        }
                    }
                };
                parts.push(format!("{} : {kind_name}", crate::print_kid_name(raw)));
            }
            QuantItem::Constraint(c) => parts.push(render_constraint(c, it)),
        }
    }
    format!("/-- Type quantifiers: {} -/", parts.join(", "))
}

/// Surface-ish rendering of a numeric constraint for the quantifier
/// comment (comments only; never elaborated).
fn render_constraint(c: &Constraint, it: &impl Interner) -> String {
    match &c.kind {
        ConstraintKind::Equal(a, b) => {
            format!("{} == {}", render_typarg(a, it), render_typarg(b, it))
        }
        ConstraintKind::NotEqual(a, b) => {
            format!("{} != {}", render_typarg(a, it), render_typarg(b, it))
        }
        ConstraintKind::Ge(a, b) => {
            format!("{} >= {}", render_nexp_surf(a, it), render_nexp_surf(b, it))
        }
        ConstraintKind::Gt(a, b) => {
            format!("{} > {}", render_nexp_surf(a, it), render_nexp_surf(b, it))
        }
        ConstraintKind::Le(a, b) => {
            format!("{} <= {}", render_nexp_surf(a, it), render_nexp_surf(b, it))
        }
        ConstraintKind::Lt(a, b) => {
            format!("{} < {}", render_nexp_surf(a, it), render_nexp_surf(b, it))
        }
        ConstraintKind::Set { nexp, members } => {
            format!(
                "{} in {{{}}}",
                render_nexp_surf(nexp, it),
                members.join(", ")
            )
        }
        ConstraintKind::And(a, b) => format!(
            "({} & {})",
            render_constraint(a, it),
            render_constraint(b, it)
        ),
        ConstraintKind::Or(a, b) => format!(
            "({} | {})",
            render_constraint(a, it),
            render_constraint(b, it)
        ),
        ConstraintKind::Not(a) => format!("not {}", render_constraint(a, it)),
        ConstraintKind::App { func, args } => format!(
            "{}({})",
            it.resolve(func.symbol).unwrap_or("<unresolvable>"),
            args.iter()
                .map(|a| render_typarg(a, it))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ConstraintKind::Id(name) => it
            .resolve(name.symbol)
            .unwrap_or("<unresolvable>")
            .to_owned(),
        ConstraintKind::Var(sym) => {
            crate::print_kid_name(it.resolve(*sym).unwrap_or("<unresolvable>"))
        }
        ConstraintKind::True => "true".to_string(),
        ConstraintKind::False => "false".to_string(),
    }
}

fn render_typarg(a: &sail_ast::TypArg, it: &impl Interner) -> String {
    match &a.kind {
        TypArgKind::Typ(_) => "<typ>".to_string(),
        TypArgKind::NExp(n) => render_nexp_surf(n, it),
        TypArgKind::Bool(c) => render_constraint(c, it),
    }
}

/// Surface-ish rendering of a size expression for comments.
fn render_nexp_surf(n: &NExp, it: &impl Interner) -> String {
    match &n.kind {
        NExpKind::Constant(s) => s.clone(),
        NExpKind::Var(sym) => crate::print_kid_name(it.resolve(*sym).unwrap_or("<unresolvable>")),
        NExpKind::Id(name) => it
            .resolve(name.symbol)
            .unwrap_or("<unresolvable>")
            .to_owned(),
        NExpKind::Sum(a, b) => format!(
            "({} + {})",
            render_nexp_surf(a, it),
            render_nexp_surf(b, it)
        ),
        NExpKind::Minus(a, b) => format!(
            "({} - {})",
            render_nexp_surf(a, it),
            render_nexp_surf(b, it)
        ),
        NExpKind::Times(a, b) => format!(
            "({} * {})",
            render_nexp_surf(a, it),
            render_nexp_surf(b, it)
        ),
        NExpKind::Exp(a) => format!("(2 ^ {})", render_nexp_surf(a, it)),
        NExpKind::Neg(a) => format!("(- {})", render_nexp_surf(a, it)),
        NExpKind::App { func, args } => format!(
            "{}({})",
            it.resolve(func.symbol).unwrap_or("<unresolvable>"),
            args.iter()
                .map(|a| render_nexp_surf(a, it))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        NExpKind::If { .. } => "<if>".to_string(),
    }
}

impl ExtSig {
    /// Render the signature header (mirrors
    /// [`LeanPrinter::print_sig_header`][crate::LeanPrinter::print_sig_header]
    /// with `{k_…}` binders inserted between the name and the value
    /// binders).
    pub fn render_header<W: fmt::Write>(
        &self,
        printer: &LeanPrinter,
        out: &mut W,
    ) -> Result<(), Error> {
        if self.sig.name.is_empty() {
            return Err(Error::Unsupported {
                msg: "anonymous function is not translatable".to_string(),
                span: self.sig.span,
            });
        }
        if printer.options().noncomputable.contains(&self.sig.name) {
            out.write_str("noncomputable ")?;
        }
        if printer.options().partial.contains(&self.sig.name) {
            out.write_str("partial ")?;
        }
        out.write_str("def ")?;
        printer.print_id_ctor(out, &self.sig.name, false)?;
        for imp in &self.implicits {
            out.write_str(" {")?;
            out.write_str(&crate::print_kid_name(&imp.kid))?;
            out.write_str(" : ")?;
            out.write_str(imp.kind.lean_name())?;
            out.write_char('}')?;
        }
        for (name, ty) in &self.sig.args {
            out.write_str(" (")?;
            printer.print_id_ctor(out, name, false)?;
            out.write_str(" : ")?;
            printer.print_typ(out, ty)?;
            out.write_char(')')?;
        }
        out.write_str(" : ")?;
        if let Some(monad) = LeanPrinter::ret_monad_of(self.sig.is_monadic) {
            write!(out, "{monad} ")?;
        }
        printer.print_typ(out, &self.sig.ret)?;
        out.write_str(" :=")?;
        Ok(())
    }

    /// Render a complete `def` with the OCaml body frame for the
    /// `(is_monadic, early_return, has_loop)` triple. `body: None` is a
    /// hard [`Error::Unsupported`] (no silent `sorry`; same rule as
    /// [`LeanPrinter::print_fun_stub`][crate::LeanPrinter::print_fun_stub]).
    pub fn render_def<W: fmt::Write>(
        &self,
        printer: &LeanPrinter,
        out: &mut W,
        body: Option<&str>,
    ) -> Result<(), Error> {
        self.render_header(printer, out)?;
        let text = match body {
            Some(b) => b,
            None => {
                return Err(Error::Unsupported {
                    msg: format!(
                        "function `{}` body is not translatable (no silent sorry)",
                        self.sig.name
                    ),
                    span: self.sig.span,
                });
            }
        };
        let frame = LeanPrinter::function_frame_suffix(
            self.sig.is_monadic,
            self.sig.early_return,
            self.sig.has_loop,
        );
        if frame.is_empty() {
            out.write_char(' ')?;
            out.write_str(text)?;
        } else {
            out.write_char(' ')?;
            out.write_str(frame)?;
            out.write_str("\n  ")?;
            out.write_str(text)?;
        }
        out.write_char('\n')?;
        Ok(())
    }

    /// [`ExtSig::render_def`] into a fresh `String` (quantifier comment
    /// first, then the `def`).
    pub fn render(&self, printer: &LeanPrinter, body: Option<&str>) -> Result<String, Error> {
        let mut buf = String::new();
        if !self.quant_comment.is_empty() {
            buf.push_str(&self.quant_comment);
            buf.push('\n');
        }
        self.render_def(printer, &mut buf, body)?;
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{FileId, Name, Node, SimpleInterner, Typ as AstTy};

    fn span() -> Span {
        Span::new(FileId(0), 0, 1)
    }

    fn pure() -> EffectFlags {
        EffectFlags::default()
    }

    fn id_typ(it: &mut SimpleInterner, name: &str) -> AstTy {
        Node::dummy(TypKind::Id(Name::dummy(it.intern(name))))
    }

    fn var_nexp(it: &mut SimpleInterner, text: &str) -> NExp {
        let sym = it.intern(text);
        Node::dummy(NExpKind::Var(sym))
    }

    fn const_nexp(text: &str) -> NExp {
        Node::dummy(NExpKind::Constant(text.to_owned()))
    }

    fn atom_app(it: &mut SimpleInterner, arg: NExp) -> AstTy {
        let targ = Node::dummy(TypArgKind::NExp(arg));
        let name = Name::dummy(it.intern("atom"));
        Node::dummy(TypKind::App {
            name,
            args: vec![targ],
        })
    }

    fn atom_bool_app(it: &mut SimpleInterner, text: &str) -> AstTy {
        let sym = it.intern(text);
        let c = Node::dummy(ConstraintKind::Var(sym));
        let targ = Node::dummy(TypArgKind::Bool(Box::new(c)));
        let name = Name::dummy(it.intern("atom_bool"));
        Node::dummy(TypKind::App {
            name,
            args: vec![targ],
        })
    }

    fn bits_app(it: &mut SimpleInterner, arg: NExp) -> AstTy {
        let targ = Node::dummy(TypArgKind::NExp(arg));
        let name = Name::dummy(it.intern("bits"));
        Node::dummy(TypKind::App {
            name,
            args: vec![targ],
        })
    }

    fn range_app(it: &mut SimpleInterner, lo: NExp) -> AstTy {
        let hi = const_nexp("31");
        let lo_arg = Node::dummy(TypArgKind::NExp(lo));
        let hi_arg = Node::dummy(TypArgKind::NExp(hi));
        let name = Name::dummy(it.intern("range"));
        Node::dummy(TypKind::App {
            name,
            args: vec![lo_arg, hi_arg],
        })
    }

    fn fn_scheme(args: Vec<AstTy>, ret: AstTy) -> TypSchm {
        TypSchm {
            quant: Vec::new(),
            typ: Node::dummy(TypKind::Fn {
                args,
                ret: Box::new(ret),
            }),
        }
    }

    fn qid(kind: Kind, sym: sail_ast::Symbol) -> QuantItem {
        QuantItem::Id(KOpt { kind, kid: sym })
    }

    fn ge(a: NExp, b: NExp) -> Constraint {
        Node::dummy(ConstraintKind::Ge(a, b))
    }

    fn le(a: NExp, b: NExp) -> Constraint {
        Node::dummy(ConstraintKind::Le(a, b))
    }

    // -- typ_from_ast_ext: atom / atom_bool / range -----------------------

    #[test]
    fn atom_var_without_evidence_is_int() {
        let mut it = SimpleInterner::new();
        let n = var_nexp(&mut it, "'d");
        let t = atom_app(&mut it, n);
        assert_eq!(typ_from_ast_ext(&t, &it, &BTreeSet::new()), Some(Typ::Int));
    }

    #[test]
    fn atom_var_with_nonneg_evidence_is_nat() {
        let mut it = SimpleInterner::new();
        let n = var_nexp(&mut it, "'d");
        let t = atom_app(&mut it, n);
        let mut nonneg = BTreeSet::new();
        nonneg.insert("'d".to_string());
        assert_eq!(typ_from_ast_ext(&t, &it, &nonneg), Some(Typ::Nat));
    }

    #[test]
    fn atom_constant_nonneg_is_nat_negative_is_int() {
        let mut it = SimpleInterner::new();
        let t_pos = atom_app(&mut it, const_nexp("31"));
        assert_eq!(
            typ_from_ast_ext(&t_pos, &it, &BTreeSet::new()),
            Some(Typ::Nat)
        );
        let t_neg = atom_app(&mut it, const_nexp("-3"));
        assert_eq!(
            typ_from_ast_ext(&t_neg, &it, &BTreeSet::new()),
            Some(Typ::Int)
        );
    }

    #[test]
    fn atom_bool_is_bool_regardless_of_payload() {
        let mut it = SimpleInterner::new();
        let t = atom_bool_app(&mut it, "'neg");
        assert_eq!(typ_from_ast_ext(&t, &it, &BTreeSet::new()), Some(Typ::Bool));
    }

    #[test]
    fn range_lower_bound_decides_nat_vs_int() {
        let mut it = SimpleInterner::new();
        let nonneg = BTreeSet::new();
        let t_nat = range_app(&mut it, const_nexp("0"));
        assert_eq!(typ_from_ast_ext(&t_nat, &it, &nonneg), Some(Typ::Nat));
        let t_int = range_app(&mut it, const_nexp("-8"));
        assert_eq!(typ_from_ast_ext(&t_int, &it, &nonneg), Some(Typ::Int));
    }

    #[test]
    fn plain_types_still_delegate_to_conv() {
        let mut it = SimpleInterner::new();
        let t = id_typ(&mut it, "bool");
        assert_eq!(typ_from_ast_ext(&t, &it, &BTreeSet::new()), Some(Typ::Bool));
        let unknown: AstTy = Node::dummy(TypKind::Unknown);
        assert_eq!(typ_from_ast_ext(&unknown, &it, &BTreeSet::new()), None);
    }

    // -- nonneg_kids --------------------------------------------------------

    #[test]
    fn nonneg_collects_ge_and_le() {
        let mut it = SimpleInterner::new();
        let sym_a = it.intern("'a");
        let sym_b = it.intern("'b");
        let na = var_nexp(&mut it, "'a");
        let nb = var_nexp(&mut it, "'b");
        let unit = id_typ(&mut it, "unit");
        let scheme = TypSchm {
            quant: vec![
                qid(Kind::Int, sym_a),
                qid(Kind::Int, sym_b),
                QuantItem::Constraint(ge(na, const_nexp("0"))),
                QuantItem::Constraint(le(const_nexp("0"), nb)),
            ],
            typ: unit,
        };
        let nonneg = nonneg_kids(&scheme, &it);
        assert!(nonneg.contains("'a"));
        assert!(nonneg.contains("'b"));
    }

    #[test]
    fn nonneg_ignores_negative_evidence_and_disjunctions() {
        let mut it = SimpleInterner::new();
        let na = var_nexp(&mut it, "'a");
        let nb1 = var_nexp(&mut it, "'b");
        let nb2 = var_nexp(&mut it, "'b");
        let disj = Node::dummy(ConstraintKind::Or(
            Box::new(ge(nb1, const_nexp("0"))),
            Box::new(ge(nb2, const_nexp("5"))),
        ));
        let unit = id_typ(&mut it, "unit");
        let scheme = TypSchm {
            quant: vec![
                QuantItem::Constraint(ge(na, const_nexp("-1"))),
                QuantItem::Constraint(disj),
            ],
            typ: unit,
        };
        let nonneg = nonneg_kids(&scheme, &it);
        assert!(!nonneg.contains("'a"));
        assert!(!nonneg.contains("'b"));
    }

    #[test]
    fn nonneg_collects_set_and_equal() {
        let mut it = SimpleInterner::new();
        let sym_s = it.intern("'s");
        let sym_e = it.intern("'e");
        let ns = var_nexp(&mut it, "'s");
        let set = Node::dummy(ConstraintKind::Set {
            nexp: ns,
            members: vec!["8".to_string(), "16".to_string()],
        });
        let ne = var_nexp(&mut it, "'e");
        let targ_l = Node::dummy(TypArgKind::NExp(ne));
        let const5 = const_nexp("5");
        let targ_r = Node::dummy(TypArgKind::NExp(const5));
        let eq = Node::dummy(ConstraintKind::Equal(Box::new(targ_l), Box::new(targ_r)));
        let unit = id_typ(&mut it, "unit");
        let scheme = TypSchm {
            quant: vec![
                qid(Kind::Int, sym_s),
                qid(Kind::Int, sym_e),
                QuantItem::Constraint(set),
                QuantItem::Constraint(eq),
            ],
            typ: unit,
        };
        let nonneg = nonneg_kids(&scheme, &it);
        assert!(nonneg.contains("'s"));
        assert!(nonneg.contains("'e"));
    }

    // -- ext_sig_from_scheme: ARM execute-like shape ------------------------

    /// `forall 'd 'datasize 'n, (0 <= 'n & 0 <= 'd).
    /// (int('d), int('datasize), int('n)) -> unit` — the dominant ARM
    /// execute form.
    #[test]
    fn arm_execute_shape_lowers_without_implicits() {
        let mut it = SimpleInterner::new();
        let sym_d = it.intern("'d");
        let sym_ds = it.intern("'datasize");
        let sym_n = it.intern("'n");
        let nn = var_nexp(&mut it, "'n");
        let nd = var_nexp(&mut it, "'d");
        let constraint = Node::dummy(ConstraintKind::And(
            Box::new(ge(nn, const_nexp("0"))),
            Box::new(ge(nd, const_nexp("0"))),
        ));
        let __n0 = var_nexp(&mut it, "'d");
        let ad = atom_app(&mut it, __n0);
        let __n1 = var_nexp(&mut it, "'datasize");
        let ads = atom_app(&mut it, __n1);
        let __n2 = var_nexp(&mut it, "'n");
        let an = atom_app(&mut it, __n2);
        let ret = id_typ(&mut it, "unit");
        let scheme = TypSchm {
            quant: vec![
                qid(Kind::Int, sym_d),
                qid(Kind::Int, sym_ds),
                qid(Kind::Int, sym_n),
                QuantItem::Constraint(constraint),
            ],
            typ: Node::dummy(TypKind::Fn {
                args: vec![ad, ads, an],
                ret: Box::new(ret),
            }),
        };
        let binders = ["d", "datasize", "n"].map(str::to_owned);
        let ext = ext_sig_from_scheme(
            "execute_op",
            SigKind::Function,
            &scheme,
            &binders,
            pure(),
            span(),
            &it,
        )
        .unwrap();
        // `atom(…)` payloads do not survive lowering, so no kind var is
        // used in the signature: no implicit binders needed.
        assert!(ext.implicits.is_empty());
        assert!(ext.quant_comment.contains("Type quantifiers"));
        assert!(ext.quant_comment.contains("k_n"));
        assert_eq!(ext.sig.args[0].1, Typ::Nat);
        // 'datasize has no nonneg evidence here → Int (sound fallback).
        assert_eq!(ext.sig.args[1].1, Typ::Int);
        let printer = LeanPrinter::with_default();
        let got = ext.render(&printer, Some("()")).unwrap();
        assert!(
            got.contains("def execute_op (d : Nat) (datasize : Int) (n : Nat) : Unit := ()"),
            "{got}"
        );
        assert!(!got.contains("sorry"));
        assert!(!got.contains("admit"));
    }

    /// `forall 'm. bits('m) -> bits('m)`: used kind var → binder.
    #[test]
    fn used_kind_var_gets_an_implicit_binder() {
        let mut it = SimpleInterner::new();
        let sym_m = it.intern("'m");
        let __n3 = var_nexp(&mut it, "'m");
        let dom = bits_app(&mut it, __n3);
        let __n4 = var_nexp(&mut it, "'m");
        let ret = bits_app(&mut it, __n4);
        let scheme = TypSchm {
            quant: vec![qid(Kind::Int, sym_m)],
            typ: Node::dummy(TypKind::Fn {
                args: vec![dom],
                ret: Box::new(ret),
            }),
        };
        let ext = ext_sig_from_scheme(
            "dup_bits",
            SigKind::Function,
            &scheme,
            &["x".to_string()],
            pure(),
            span(),
            &it,
        )
        .unwrap();
        assert_eq!(ext.implicits.len(), 1);
        assert_eq!(ext.implicits[0].kind, ImplicitKind::Int);
        let printer = LeanPrinter::with_default();
        let got = ext.render(&printer, Some("x")).unwrap();
        assert!(got.contains("{k_m : Int}"), "{got}");
        assert!(got.contains("(BitVec k_m)"), "{got}");
    }

    /// `forall ('a : Type). ('a, 'a) -> bool` (`eq_any` shape).
    #[test]
    fn type_kind_var_gets_a_type_binder() {
        let mut it = SimpleInterner::new();
        let sym_a = it.intern("'a");
        let sym_x = it.intern("'a");
        let sym_y = it.intern("'a");
        let vx: AstTy = Node::dummy(TypKind::Var(sym_x));
        let vy: AstTy = Node::dummy(TypKind::Var(sym_y));
        let ret = id_typ(&mut it, "bool");
        let scheme = TypSchm {
            quant: vec![qid(Kind::Type, sym_a)],
            typ: Node::dummy(TypKind::Fn {
                args: vec![vx, vy],
                ret: Box::new(ret),
            }),
        };
        let ext = ext_sig_from_scheme(
            "eq_any",
            SigKind::Value,
            &scheme,
            &["x".to_string(), "y".to_string()],
            pure(),
            span(),
            &it,
        )
        .unwrap();
        assert_eq!(ext.implicits.len(), 1);
        assert_eq!(ext.implicits[0].kind, ImplicitKind::Type);
        let printer = LeanPrinter::with_default();
        let got = ext.render(&printer, Some("true")).unwrap();
        assert!(got.contains("{k_a : Type}"), "{got}");
    }

    /// Bool-kind quantifier with `bool('b)` domain (`ArchitectureReset`).
    #[test]
    fn bool_singleton_domain_lowers_to_bool() {
        let mut it = SimpleInterner::new();
        let sym_cold = it.intern("'cold");
        let dom = atom_bool_app(&mut it, "'cold");
        let ret = id_typ(&mut it, "unit");
        let scheme = TypSchm {
            quant: vec![qid(Kind::Bool, sym_cold)],
            typ: Node::dummy(TypKind::Fn {
                args: vec![dom],
                ret: Box::new(ret),
            }),
        };
        let ext = ext_sig_from_scheme(
            "reset",
            SigKind::Function,
            &scheme,
            &["cold".to_string()],
            pure(),
            span(),
            &it,
        )
        .unwrap();
        assert_eq!(ext.sig.args[0].1, Typ::Bool);
        assert!(ext.implicits.is_empty());
        let printer = LeanPrinter::with_default();
        let got = ext.render(&printer, Some("()")).unwrap();
        assert!(got.contains("def reset (cold : Bool) : Unit"), "{got}");
    }

    /// `integer_subrange` shape: arithmetic return width keeps its vars.
    #[test]
    fn arithmetic_return_width_keeps_vars_and_comment() {
        let mut it = SimpleInterner::new();
        let sym_hi = it.intern("'hi");
        let sym_lo = it.intern("'lo");
        let nhi = var_nexp(&mut it, "'hi");
        let nlo = var_nexp(&mut it, "'lo");
        let width = Node::dummy(NExpKind::Sum(
            Box::new(Node::dummy(NExpKind::Minus(Box::new(nhi), Box::new(nlo)))),
            Box::new(const_nexp("1")),
        ));
        let ret = bits_app(&mut it, width);
        let clo = var_nexp(&mut it, "'lo");
        let chi = var_nexp(&mut it, "'hi");
        let le_c = le(clo, chi);
        let plain_int = id_typ(&mut it, "int");
        let __n5 = var_nexp(&mut it, "'hi");
        let ahi = atom_app(&mut it, __n5);
        let __n6 = var_nexp(&mut it, "'lo");
        let alo = atom_app(&mut it, __n6);
        let scheme = TypSchm {
            quant: vec![
                qid(Kind::Int, sym_hi),
                qid(Kind::Int, sym_lo),
                QuantItem::Constraint(le_c),
            ],
            typ: Node::dummy(TypKind::Fn {
                args: vec![plain_int, ahi, alo],
                ret: Box::new(ret),
            }),
        };
        let ext = ext_sig_from_scheme(
            "integer_subrange",
            SigKind::Value,
            &scheme,
            &["x".to_string(), "hi".to_string(), "lo".to_string()],
            pure(),
            span(),
            &it,
        )
        .unwrap();
        assert_eq!(ext.implicits.len(), 2);
        assert!(ext.quant_comment.contains("k_lo"));
        let printer = LeanPrinter::with_default();
        let got = ext.render(&printer, Some("x")).unwrap();
        // NOTE (latent lib.rs wart, docking spec item 5): top-level
        // `Sum`/`Minus` sizes print without parens inside `(BitVec …)`,
        // so Lean parses `(BitVec k_hi) - k_lo + 1` and rejects the `def`
        // loudly at elaboration. Pre-existing `print_typ` behaviour for
        // every compound size — not papered over here.
        assert!(got.contains("(BitVec k_hi - k_lo + 1)"), "{got}");
    }

    // -- precise failures ----------------------------------------------------

    #[test]
    fn unknown_stays_unsupported() {
        let it = SimpleInterner::new();
        let scheme = TypSchm {
            quant: Vec::new(),
            typ: Node::dummy(TypKind::Unknown),
        };
        let err = ext_sig_from_scheme("f", SigKind::Value, &scheme, &[], pure(), span(), &it)
            .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }));
        assert!(err.to_string().contains("Unknown"));
    }

    #[test]
    fn implicit_binder_stays_unsupported() {
        let mut it = SimpleInterner::new();
        let n = var_nexp(&mut it, "'n");
        let targ = Node::dummy(TypArgKind::NExp(n));
        let sym_implicit = it.intern("implicit");
        let imp: AstTy = Node::dummy(TypKind::App {
            name: Name::dummy(sym_implicit),
            args: vec![targ],
        });
        let __n7 = var_nexp(&mut it, "'n");
        let ret = bits_app(&mut it, __n7);
        let scheme = fn_scheme(vec![imp], ret);
        let err = ext_sig_from_scheme(
            "Ones",
            SigKind::Function,
            &scheme,
            &["n".to_string()],
            pure(),
            span(),
            &it,
        )
        .unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }));
        assert!(err.to_string().contains("implicit"), "{err}");
    }

    #[test]
    fn existential_stays_unsupported() {
        let mut it = SimpleInterner::new();
        let sym_n = it.intern("'n");
        let inner = id_typ(&mut it, "int");
        let tru = Node::dummy(ConstraintKind::True);
        let scheme = TypSchm {
            quant: Vec::new(),
            typ: Node::dummy(TypKind::Exist {
                binders: vec![KOpt {
                    kind: Kind::Int,
                    kid: sym_n,
                }],
                constraint: Box::new(tru),
                typ: Box::new(inner),
            }),
        };
        let err = ext_sig_from_scheme("f", SigKind::Value, &scheme, &[], pure(), span(), &it)
            .unwrap_err();
        assert!(err.to_string().contains("existential"), "{err}");
    }

    #[test]
    fn bidir_top_level_points_at_bidir_sigs() {
        let mut it = SimpleInterner::new();
        let lhs = id_typ(&mut it, "int");
        let rhs = id_typ(&mut it, "bool");
        let scheme = TypSchm {
            quant: Vec::new(),
            typ: Node::dummy(TypKind::Bidir {
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            }),
        };
        let err = ext_sig_from_scheme("m", SigKind::Value, &scheme, &[], pure(), span(), &it)
            .unwrap_err();
        assert!(err.to_string().contains("bidir_sigs"), "{err}");
    }

    #[test]
    fn bidir_sigs_splits_forwards_and_backwards() {
        let mut it = SimpleInterner::new();
        let lhs = id_typ(&mut it, "int");
        let rhs = id_typ(&mut it, "bool");
        let (fwd, bwd) = bidir_sigs(
            "conv",
            &lhs,
            &rhs,
            &["x".to_string()],
            &["y".to_string()],
            pure(),
            span(),
            &it,
        )
        .unwrap();
        assert_eq!(fwd.name, "conv_forwards");
        assert_eq!(bwd.name, "conv_backwards");
        assert_eq!(fwd.args.len(), 1);
        assert_eq!(fwd.ret, Typ::Bool);
        assert_eq!(bwd.args.len(), 1);
        assert_eq!(bwd.ret, Typ::Int);
    }

    #[test]
    fn bidir_sigs_untuples_a_tuple_side() {
        let mut it = SimpleInterner::new();
        let a = id_typ(&mut it, "int");
        let b = id_typ(&mut it, "bool");
        let lhs: AstTy = Node::dummy(TypKind::Tuple(vec![a, b]));
        let rhs = id_typ(&mut it, "unit");
        let (fwd, _) = bidir_sigs(
            "m",
            &lhs,
            &rhs,
            &["a".to_string(), "b".to_string()],
            &["u".to_string()],
            pure(),
            span(),
            &it,
        )
        .unwrap();
        assert_eq!(fwd.args.len(), 2);
    }

    #[test]
    fn anonymous_arity_and_register_are_precise() {
        let mut it = SimpleInterner::new();
        let dom = id_typ(&mut it, "int");
        let ret = id_typ(&mut it, "int");
        let scheme = fn_scheme(vec![dom], ret);
        let err = ext_sig_from_scheme(
            "",
            SigKind::Function,
            &scheme,
            &["x".to_string()],
            pure(),
            span(),
            &it,
        )
        .unwrap_err();
        assert!(err.to_string().contains("anonymous"), "{err}");
        assert!(matches!(
            classify_scheme(SigKind::Function, &scheme, &[], &it),
            SigForm::ArityMismatch {
                domains: 1,
                binders: 0
            }
        ));
        let plain = TypSchm {
            quant: Vec::new(),
            typ: id_typ(&mut it, "bool"),
        };
        let err = ext_sig_from_scheme("r", SigKind::Register, &plain, &[], pure(), span(), &it)
            .unwrap_err();
        assert!(err.to_string().contains("register"), "{err}");
        assert_eq!(
            classify_scheme(SigKind::Register, &plain, &[], &it),
            SigForm::RegisterAsDef
        );
    }

    #[test]
    fn classify_covers_the_inventory_classes() {
        let mut it = SimpleInterner::new();
        let dom = id_typ(&mut it, "int");
        let ret = id_typ(&mut it, "bool");
        let mono = fn_scheme(vec![dom], ret);
        assert_eq!(
            classify_scheme(SigKind::Function, &mono, &["x".to_string()], &it),
            SigForm::PlainMono
        );
        let sym_n = it.intern("'n");
        let q = TypSchm {
            quant: vec![qid(Kind::Int, sym_n)],
            typ: mono.typ.clone(),
        };
        assert!(matches!(
            classify_scheme(SigKind::Function, &q, &["x".to_string()], &it),
            SigForm::Quantified {
                vars: 1,
                constraints: 0
            }
        ));
        let inner_dom = id_typ(&mut it, "int");
        let inner_ret = id_typ(&mut it, "int");
        let higher: AstTy = Node::dummy(TypKind::Fn {
            args: vec![inner_dom],
            ret: Box::new(inner_ret),
        });
        let outer_ret = id_typ(&mut it, "int");
        let nested = fn_scheme(vec![higher], outer_ret);
        assert!(matches!(
            classify_scheme(SigKind::Function, &nested, &["f".to_string()], &it),
            SigForm::TypeFailure(SigFail::NestedFn)
        ));
        let plain_fn = TypSchm {
            quant: Vec::new(),
            typ: id_typ(&mut it, "int"),
        };
        assert_eq!(
            classify_scheme(SigKind::Function, &plain_fn, &[], &it),
            SigForm::NotAFunction
        );
    }

    // -- rendering: frames, options, no holes ---------------------------------

    #[test]
    fn effect_flags_drive_monad_and_frame() {
        let mut it = SimpleInterner::new();
        let dom = id_typ(&mut it, "int");
        let ret = id_typ(&mut it, "int");
        let scheme = fn_scheme(vec![dom], ret);
        let ext = ext_sig_from_scheme(
            "f",
            SigKind::Function,
            &scheme,
            &["x".to_string()],
            EffectFlags {
                is_monadic: true,
                early_return: true,
                has_loop: false,
            },
            span(),
            &it,
        )
        .unwrap();
        let printer = LeanPrinter::with_default();
        let got = ext.render(&printer, Some("x")).unwrap();
        assert_eq!(
            got, "def f (x : Int) : SailM Int := SailME.run do\n  x\n",
            "{got}"
        );
    }

    #[test]
    fn missing_body_is_refused_never_sorry() {
        let mut it = SimpleInterner::new();
        let plain = TypSchm {
            quant: Vec::new(),
            typ: id_typ(&mut it, "nat"),
        };
        let ext =
            ext_sig_from_scheme("v", SigKind::Value, &plain, &[], pure(), span(), &it).unwrap();
        let printer = LeanPrinter::with_default();
        let err = ext.render(&printer, None).unwrap_err();
        assert_eq!(
            err,
            Error::Unsupported {
                msg: "function `v` body is not translatable (no silent sorry)".to_string(),
                span: span(),
            }
        );
    }

    #[test]
    fn options_prefixes_survive_extension() {
        use std::collections::BTreeSet;
        let mut it = SimpleInterner::new();
        let plain = TypSchm {
            quant: Vec::new(),
            typ: id_typ(&mut it, "nat"),
        };
        let ext =
            ext_sig_from_scheme("nc", SigKind::Value, &plain, &[], pure(), span(), &it).unwrap();
        let opts = crate::Options {
            noncomputable: BTreeSet::from(["nc".to_string()]),
            ..crate::Options::default()
        };
        let printer = LeanPrinter::new(opts);
        let got = ext.render(&printer, Some("1")).unwrap();
        assert_eq!(got, "noncomputable def nc : Nat := 1\n", "{got}");
    }
}
