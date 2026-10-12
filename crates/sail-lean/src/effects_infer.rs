#![forbid(unsafe_code)]
//! Lane 68: `EffectFlags` inference from real effect info.
//!
//! Derives the `(is_monadic, early_return, has_loop)` triple per definition
//! from the canonical [`sail_ast`] tree plus the [`sail_effects`] predicates.
//! Sound-conservative contract: **unknown ⇒ monadic** — anything this module
//! cannot prove pure sets `is_monadic = true` instead of guessing.
//!
//! ## Semantic masters
//!
//! - `is_monadic` mirrors OCaml `not (Effects.function_is_pure …)` as
//!   modelled by [`sail_effects::function_is_pure`]: register / memory /
//!   nondeterminism effects and `throw` make a body monadic. Calls to
//!   functions outside the known-pure set (see [`EffectEnv::pure_fns`]) are
//!   monadic — this covers unknown externs and not-yet-lowered callees.
//! - `early_return` mirrors [`sail_effects::has_early_return`]: any `return`
//!   (surface `early_return` app or `E_return`), `throw`, `exit` or `assert`
//!   anywhere in the body sets it. This deliberately over-approximates the
//!   tail-`return` refinement of the statement IR (any explicit `return`
//!   counts, even in tail position): the extra `ExceptM`/`SailME` frame is
//!   always sound, a missing one is not.
//!
//!   Note the intentional divergence from `sail-except::has_early_return`,
//!   which counts only `return` and excludes `throw`/`exit`: the reference
//!   for *this* lane is the `sail-effects` crate (task spec), whose
//!   `effect_label` maps `throw`/`assert` to the `ExceptM` layer.
//! - `has_loop` mirrors [`sail_effects::has_loop`]: a direct `Loop`/`For`
//!   node anywhere in the body sets it. Calls never propagate `has_loop`
//!   (same as OCaml `has_loop exp`, which is syntactic per body).
//!
//! ## Soundness rules over `sail_ast`
//!
//! - Bound locals (function parameters, `let`/`plet` patterns, `var`
//!   declarations, `for` indices, match-arm patterns) are pure.
//! - A free `Id` that is neither bound nor otherwise classified is
//!   **monadic**: it may denote a register or a global. Callers should fill
//!   [`EffectEnv::registers`] and [`EffectEnv::pure_fns`] (from `register`
//!   declarations and `val`/`extern pure` specs); without that info the
//!   inference stays sound by over-approximating.
//! - `Ref` (register reference), `Deref` (memory), `Undef` (unknown /
//!   nondeterministic value) and writes to non-local `LExp` targets are
//!   monadic.
//! - All matches are exhaustive (no wildcards) so new AST variants fail to
//!   compile here instead of silently defaulting.
//!
//! ## Andock-Spec für `emit.rs` / `exps.rs` (NICHT in dieser Lane editieren)
//!
//! Stand der Referenz (Haupt-Worktree, nur gelesen):
//! - `emit.rs::EffectFlags` ist feldgleich zu [`EffectFlags`] hier
//!   (`is_monadic`, `early_return`, `has_loop`, gleiche Dok-Zeilen) und
//!   `funsig_from_scheme(..., effects: EffectFlags, ...)` nimmt die Flags
//!   **explizit** — der Emitter rät nie, er übernimmt.
//! - `sail-ty::TypedDef` trägt `kind: TypedDefKind` (`Function`/`Value`/…)
//!   plus `interner()`-Zugriff auf den `SimpleInterner`.
//!
//! Verdrahtung (Owner der Emit-Lane):
//! - Pro `TypedDef` mit `TypedDefKind::Function | TypedDefKind::Value` die
//!   Klausel-Bodies mit [`infer_def`] (oder programmweit [`infer_program`]
//!   für transitive `is_monadic`-Schlüsse) auswerten und das Ergebnis 1:1
//!   an `funsig_from_scheme` durchreichen (`FunSig`-Felder 1:1).
//! - Fehlt Body oder Interner, oder ist die Definition nicht ausführbar
//!   (kein Eintrag aus [`infer_def`]), [`EffectFlags::UNKNOWN`] verwenden
//!   (alle `true`), niemals `PURE`.
//! - `EffectEnv` aus vorhandenen Fakten befüllen: `registers` aus
//!   `register`-Deklarationen (`TypedDefKind::Register`), `pure_fns` aus
//!   `val`-Specs mit `extern pure` plus lokal bewiesenen reinen Funktionen.
//!   Nichts davon wird hier geraten.
//! - Abgrenzung zu `exps.rs` (eigene Lane): dort feingranulare
//!   Exp-Prädikate (`exp_has_effect`/`exp_has_early_return`/`exp_has_loop`);
//!   hier die konservative Def-Ebene mit transitivem Abschluss plus
//!   [`sail_effects`]-Brücke ([`EffectFlags::from_ir`], [`EffectFlags::from_label`]).
//!   Beide Seiten müssen die `throw`/`assert`-Abbildung gleich halten
//!   (diese Lane folgt `sail-effects`: `throw`/`exit` ⇒ monadisch + early,
//!   `assert` ⇒ nur early).

use std::collections::{BTreeMap, BTreeSet};

use sail_ast::{
    Def, DefKind, Exp, ExpKind, Funcl, Interner, LExp, LExpKind, MPExpKind, MapclKind, Name, PExp,
    PExpKind, Pat, PatKind,
};

/// `(is_monadic, early_return, has_loop)` per function body.
///
/// Maps 1:1 onto `FunSig`'s three predicates (`doc_funcl_init`, OCaml
/// `pretty_print_lean.ml` ~lines 1160-1162); see [`crate::FunSig`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct EffectFlags {
    /// `not (Effects.function_is_pure …)`: return type gains `SailM`.
    pub is_monadic: bool,
    /// Body frame gains the `ExceptM` layer (`SailME.run` / `ExceptM.run`).
    pub early_return: bool,
    /// Pure bodies gain the `Id.run` frame.
    pub has_loop: bool,
}

impl EffectFlags {
    /// No effects: plain Lean value.
    pub const PURE: Self = Self {
        is_monadic: false,
        early_return: false,
        has_loop: false,
    };

    /// Sound default for unknown bodies: monadic everywhere.
    ///
    /// Over-approximating `early_return`/`has_loop` only adds wrapper frames
    /// (`SailME.run do`), which always typecheck when the plain form would;
    /// under-approximating would silently drop needed frames.
    pub const UNKNOWN: Self = Self {
        is_monadic: true,
        early_return: true,
        has_loop: true,
    };

    /// Combine two flag sets (logical OR per field).
    #[must_use]
    pub fn or(self, other: Self) -> Self {
        Self {
            is_monadic: self.is_monadic || other.is_monadic,
            early_return: self.early_return || other.early_return,
            has_loop: self.has_loop || other.has_loop,
        }
    }

    /// True iff the body is a plain Lean value (no monad, no frame).
    #[must_use]
    pub fn is_pure(self) -> bool {
        self == Self::PURE
    }

    /// `(is_monadic, early_return, has_loop)` triple for `FunSig` / frame dispatch.
    #[must_use]
    pub fn triple(self) -> (bool, bool, bool) {
        (self.is_monadic, self.early_return, self.has_loop)
    }

    /// Derive flags from an already-lowered [`sail_effects`] body.
    ///
    /// Thin wrapper over the `sail-effects` predicates, using the same
    /// `is_monadic = !function_is_pure` mapping as OCaml `doc_funcl_init`.
    #[must_use]
    pub fn from_ir(fun: &sail_effects::FunDef) -> Self {
        Self {
            is_monadic: !sail_effects::function_is_pure(fun),
            early_return: sail_effects::has_early_return(fun),
            has_loop: sail_effects::has_loop(fun),
        }
    }

    /// Build flags from a coarse [`sail_effects::EffectLabel`] plus the two
    /// independent syntactic predicates.
    ///
    /// The label alone cannot spell pure-with-early-return (same limitation
    /// as the legacy [`crate::Effect`] enum); callers must pass the
    /// independently computed `early_return`/`has_loop`, exactly like
    /// `FunSig` does.
    #[must_use]
    pub fn from_label(
        label: sail_effects::EffectLabel,
        early_return: bool,
        has_loop: bool,
    ) -> Self {
        Self {
            is_monadic: label != sail_effects::EffectLabel::Pure,
            early_return,
            has_loop,
        }
    }
}

/// Known facts the inference may use without guessing.
///
/// Everything not covered here stays conservative (unknown ⇒ monadic).
/// The emit lane fills this from `register` declarations (`registers`) and
/// from `val`/`extern pure` specs plus locally proven-pure functions
/// (`pure_fns`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EffectEnv {
    /// Register names (reads/writes are `SailM` effects).
    pub registers: BTreeSet<String>,
    /// Function names proven pure (calls stay pure).
    pub pure_fns: BTreeSet<String>,
}

impl EffectEnv {
    /// Empty environment: every free id and every call is monadic.
    /// Sound, maximally imprecise.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// True if a call to `name` contributes no monadic effect by itself
    /// (argument effects still apply).
    #[must_use]
    pub fn is_pure_call(&self, name: &str) -> bool {
        self.pure_fns.contains(name)
    }

    /// True if `name` denotes a register.
    #[must_use]
    pub fn is_register(&self, name: &str) -> bool {
        self.registers.contains(name)
    }
}

/// Infer flags for one expression with an empty local scope.
#[must_use]
pub fn infer_exp(exp: &Exp, it: &impl Interner, env: &EffectEnv) -> EffectFlags {
    infer_exp_inner(exp, it, env, &BTreeSet::new(), None)
}

/// Infer flags for one function clause; returns `(name, flags)`.
/// Unresolvable names become `"<unknown>"` (never dropped silently).
#[must_use]
pub fn infer_funcl(funcl: &Funcl, it: &impl Interner, env: &EffectEnv) -> (String, EffectFlags) {
    let name = name_text(&funcl.name, it).unwrap_or("<unknown>").to_owned();
    let flags = infer_pexp(&funcl.arm, it, env, &BTreeSet::new(), None);
    (name, flags)
}

/// Infer flags per executable definition.
///
/// Returns one `(name, flags)` entry per defined function (same-name clauses
/// are OR-combined), one entry for top-level `let`/`register` initialisers,
/// and per-clause entries for `InternalMutrec`. Non-executable definitions
/// (types, constraints, specs without bodies, pragmas, measures, …) yield
/// no entries — the caller must map "no entry" to [`EffectFlags::UNKNOWN`],
/// never to `PURE`.
#[must_use]
pub fn infer_def(def: &Def, it: &impl Interner, env: &EffectEnv) -> Vec<(String, EffectFlags)> {
    match &def.kind {
        DefKind::Fundef { funcls, .. } => {
            let mut out: Vec<(String, EffectFlags)> = Vec::new();
            for funcl in funcls {
                let entry = infer_funcl(funcl, it, env);
                merge_entry(&mut out, entry);
            }
            out
        }
        DefKind::Mapdef { name, clauses, .. } => {
            let def_name = name_text(name, it).unwrap_or("<unknown>").to_owned();
            let mut flags = EffectFlags::PURE;
            let mut seen = false;
            for clause in clauses {
                seen = true;
                flags = flags.or(match &clause.kind {
                    MapclKind::Bidir {
                        forwards,
                        backwards,
                    } => infer_mpexp_flags(forwards, it, env)
                        .or(infer_mpexp_flags(backwards, it, env)),
                    MapclKind::Forwards(pexp) | MapclKind::Backwards(pexp) => {
                        infer_pexp(pexp, it, env, &BTreeSet::new(), None)
                    }
                });
            }
            if seen {
                vec![(def_name, flags)]
            } else {
                Vec::new()
            }
        }
        DefKind::Impl(funcl) => vec![infer_funcl(funcl, it, env)],
        DefKind::Let { pat, exp } => {
            let mut binders = BTreeSet::new();
            pat_binders(pat, it, &mut binders);
            let name = binders.iter().next().cloned().unwrap_or("<let>".to_owned());
            let flags = infer_exp_inner(exp, it, env, &BTreeSet::new(), None);
            vec![(name, flags)]
        }
        DefKind::Register(spec) => {
            let name = name_text(&spec.name, it).unwrap_or("<unknown>").to_owned();
            let flags = match &spec.init {
                Some(init) => infer_exp_inner(init, it, env, &BTreeSet::new(), None),
                None => EffectFlags::PURE,
            };
            vec![(name, flags)]
        }
        DefKind::InternalMutrec(sets) => {
            let mut out: Vec<(String, EffectFlags)> = Vec::new();
            for set in sets {
                for funcl in &set.funcls {
                    let entry = infer_funcl(funcl, it, env);
                    merge_entry(&mut out, entry);
                }
            }
            out
        }
        DefKind::Outcome { defs, .. } => {
            let mut out: Vec<(String, EffectFlags)> = Vec::new();
            for inner in defs {
                for entry in infer_def(inner, it, env) {
                    merge_entry(&mut out, entry);
                }
            }
            out
        }
        DefKind::Scattered(scattered) => match scattered {
            sail_ast::ScatteredDef::Funcl(funcl) => vec![infer_funcl(funcl, it, env)],
            sail_ast::ScatteredDef::Mapcl { name, clause } => {
                let def_name = name_text(name, it).unwrap_or("<unknown>").to_owned();
                let flags = match &clause.kind {
                    MapclKind::Bidir {
                        forwards,
                        backwards,
                    } => infer_mpexp_flags(forwards, it, env)
                        .or(infer_mpexp_flags(backwards, it, env)),
                    MapclKind::Forwards(pexp) | MapclKind::Backwards(pexp) => {
                        infer_pexp(pexp, it, env, &BTreeSet::new(), None)
                    }
                };
                vec![(def_name, flags)]
            }
            sail_ast::ScatteredDef::Function { .. }
            | sail_ast::ScatteredDef::Variant { .. }
            | sail_ast::ScatteredDef::Unioncl { .. }
            | sail_ast::ScatteredDef::Mapping { .. }
            | sail_ast::ScatteredDef::Enum(_)
            | sail_ast::ScatteredDef::Enumcl { .. }
            | sail_ast::ScatteredDef::End(_) => Vec::new(),
        },
        DefKind::Type(_)
        | DefKind::Constraint(_)
        | DefKind::Val(_)
        | DefKind::Instantiation { .. }
        | DefKind::Fixity { .. }
        | DefKind::Overload { .. }
        | DefKind::Default(_)
        | DefKind::Measure { .. }
        | DefKind::LoopMeasures { .. }
        | DefKind::Pragma { .. } => Vec::new(),
    }
}

/// Infer flags for a whole program with transitive `is_monadic` propagation.
///
/// Direct per-definition flags treat calls to locally defined functions
/// neutrally (arguments still count); a fixpoint then propagates `is_monadic`
/// from callees to callers over the syntactic callgraph. Calls to functions
/// that are neither defined here nor in [`EffectEnv::pure_fns`] stay monadic
/// (unknown ⇒ monadic). `early_return`/`has_loop` are per-body syntactic and
/// never propagate — matching OCaml `has_early_return exp` / `has_loop exp`.
#[must_use]
pub fn infer_program(
    defs: &[Def],
    it: &impl Interner,
    env: &EffectEnv,
) -> BTreeMap<String, EffectFlags> {
    // Defined names first so direct flags can treat calls to locally defined
    // functions neutrally (arguments still count); unknown calls stay
    // monadic. The fixpoint below only ever ADDS `is_monadic`, so starting
    // neutral for defined callees is sound.
    let defined = defined_names(defs, it);
    let mut flags: BTreeMap<String, EffectFlags> = BTreeMap::new();
    let mut callees: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for def in defs {
        for (name, f, calls) in program_def_infos(def, it, env, &defined) {
            flags
                .entry(name.clone())
                .and_modify(|e| *e = e.or(f))
                .or_insert(f);
            callees.entry(name).or_default().extend(calls);
        }
    }
    // Transitive `is_monadic` fixpoint (bounded iterations).
    for _ in 0..flags.len().saturating_add(1) {
        let mut changed = false;
        // Snapshot callee monadic-ness first (immutable borrow), then update.
        let monadic: BTreeMap<String, bool> = flags
            .iter()
            .map(|(k, v)| (k.clone(), v.is_monadic))
            .collect();
        for (name, calls) in &callees {
            if flags.get(name).map(|f| f.is_monadic).unwrap_or(false) {
                continue;
            }
            if calls
                .iter()
                .any(|c| monadic.get(c).copied().unwrap_or(false))
            {
                if let Some(entry) = flags.get_mut(name) {
                    entry.is_monadic = true;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    flags
}

/// Direct callees of an expression (resolved `App` function names).
#[must_use]
pub fn callees_of_exp(exp: &Exp, it: &impl Interner) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    collect_calls(exp, it, &mut out);
    out
}

// --- internals -----------------------------------------------------------

fn merge_entry(out: &mut Vec<(String, EffectFlags)>, entry: (String, EffectFlags)) {
    if let Some(slot) = out.iter_mut().find(|(n, _)| *n == entry.0) {
        slot.1 = slot.1.or(entry.1);
    } else {
        out.push(entry);
    }
}

fn name_text<'a>(name: &Name, it: &'a impl Interner) -> Option<&'a str> {
    it.resolve(name.symbol)
}

/// All function names defined by `defs` (grouping matches [`infer_def`]).
fn defined_names(defs: &[Def], it: &impl Interner) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for def in defs {
        match &def.kind {
            DefKind::Fundef { funcls, .. } => {
                for funcl in funcls {
                    out.insert(name_text(&funcl.name, it).unwrap_or("<unknown>").to_owned());
                }
            }
            DefKind::Mapdef { name, .. } => {
                out.insert(name_text(name, it).unwrap_or("<unknown>").to_owned());
            }
            DefKind::Impl(funcl) => {
                out.insert(name_text(&funcl.name, it).unwrap_or("<unknown>").to_owned());
            }
            DefKind::InternalMutrec(sets) => {
                for set in sets {
                    for funcl in &set.funcls {
                        out.insert(name_text(&funcl.name, it).unwrap_or("<unknown>").to_owned());
                    }
                }
            }
            DefKind::Outcome { defs, .. } => {
                out.extend(defined_names(defs, it));
            }
            DefKind::Scattered(scattered) => match scattered {
                sail_ast::ScatteredDef::Funcl(funcl) => {
                    out.insert(name_text(&funcl.name, it).unwrap_or("<unknown>").to_owned());
                }
                sail_ast::ScatteredDef::Mapcl { name, .. } => {
                    out.insert(name_text(name, it).unwrap_or("<unknown>").to_owned());
                }
                _ => {}
            },
            _ => {}
        }
    }
    out
}

/// Per-definition infos with program context (`defined` callees neutral).
fn program_def_infos(
    def: &Def,
    it: &impl Interner,
    env: &EffectEnv,
    defined: &BTreeSet<String>,
) -> Vec<(String, EffectFlags, BTreeSet<String>)> {
    let mut out = Vec::new();
    match &def.kind {
        DefKind::Fundef { funcls, .. } => {
            let mut grouped: BTreeMap<String, (EffectFlags, BTreeSet<String>)> = BTreeMap::new();
            for funcl in funcls {
                let name = name_text(&funcl.name, it).unwrap_or("<unknown>").to_owned();
                let flags = infer_pexp_ctx(&funcl.arm, it, env, &BTreeSet::new(), Some(defined));
                let mut calls = BTreeSet::new();
                collect_pexp_calls(&funcl.arm, it, &mut calls);
                grouped
                    .entry(name)
                    .and_modify(|(f, c)| {
                        *f = f.or(flags);
                        c.extend(calls.iter().cloned());
                    })
                    .or_insert((flags, calls));
            }
            for (name, (flags, calls)) in grouped {
                out.push((name, flags, calls));
            }
        }
        DefKind::InternalMutrec(sets) => {
            for set in sets {
                for funcl in &set.funcls {
                    let name = name_text(&funcl.name, it).unwrap_or("<unknown>").to_owned();
                    let flags =
                        infer_pexp_ctx(&funcl.arm, it, env, &BTreeSet::new(), Some(defined));
                    let mut calls = BTreeSet::new();
                    collect_pexp_calls(&funcl.arm, it, &mut calls);
                    out.push((name, flags, calls));
                }
            }
        }
        _ => {
            for (name, flags) in infer_def(def, it, env) {
                let mut calls = BTreeSet::new();
                collect_def_calls(def, it, &mut calls);
                out.push((name, flags, calls));
            }
        }
    }
    out
}

fn infer_mpexp_flags(mpexp: &sail_ast::MPExp, it: &impl Interner, env: &EffectEnv) -> EffectFlags {
    match &mpexp.kind {
        MPExpKind::Pat(_) => EffectFlags::PURE,
        MPExpKind::When { pat, guard } => {
            let mut scope = BTreeSet::new();
            pat_binders(pat, it, &mut scope);
            infer_exp_inner(guard, it, env, &scope, None)
        }
    }
}

fn infer_pexp(
    arm: &PExp,
    it: &impl Interner,
    env: &EffectEnv,
    scope: &BTreeSet<String>,
    defined: Option<&BTreeSet<String>>,
) -> EffectFlags {
    match &arm.kind {
        PExpKind::PatExp { pat, body } => {
            let mut inner = scope.clone();
            pat_binders(pat, it, &mut inner);
            infer_exp_inner(body, it, env, &inner, defined)
        }
        PExpKind::PatWhen { pat, guard, body } => {
            let mut inner = scope.clone();
            pat_binders(pat, it, &mut inner);
            infer_exp_inner(guard, it, env, &inner, defined)
                .or(infer_exp_inner(body, it, env, &inner, defined))
        }
    }
}

fn infer_pexp_ctx(
    arm: &PExp,
    it: &impl Interner,
    env: &EffectEnv,
    scope: &BTreeSet<String>,
    defined: Option<&BTreeSet<String>>,
) -> EffectFlags {
    infer_pexp(arm, it, env, scope, defined)
}

fn infer_exp_inner(
    exp: &Exp,
    it: &impl Interner,
    env: &EffectEnv,
    scope: &BTreeSet<String>,
    defined: Option<&BTreeSet<String>>,
) -> EffectFlags {
    match &exp.kind {
        ExpKind::Block(es) => combine_all(es, it, env, scope, defined),
        ExpKind::Id(name) => id_flags(name, it, env, scope),
        ExpKind::Lit(_) => EffectFlags::PURE,
        ExpKind::Typ { exp, .. } => infer_exp_inner(exp, it, env, scope, defined),
        ExpKind::App { func, args } => {
            let mut flags = combine_all(args, it, env, scope, defined);
            let func_name = name_text(func, it);
            match func_name {
                // Elaborated surface `return e` (cf. sail-except detect).
                Some("early_return") => flags.early_return = true,
                Some(other) => {
                    let known_defined = defined.map(|d| d.contains(other)).unwrap_or(false);
                    if !known_defined && !env.is_pure_call(other) {
                        // Unknown callee: may touch registers / throw.
                        flags.is_monadic = true;
                    }
                }
                // Unresolvable callee: unknown ⇒ monadic.
                None => flags.is_monadic = true,
            }
            flags
        }
        ExpKind::Tuple(es) | ExpKind::Vector(es) | ExpKind::List(es) => {
            combine_all(es, it, env, scope, defined)
        }
        ExpKind::If {
            cond,
            then_branch,
            else_branch,
        } => infer_exp_inner(cond, it, env, scope, defined)
            .or(infer_exp_inner(then_branch, it, env, scope, defined))
            .or(infer_exp_inner(else_branch, it, env, scope, defined)),
        ExpKind::Loop {
            measure,
            cond,
            body,
            ..
        } => {
            let mut flags = EffectFlags {
                is_monadic: false,
                early_return: false,
                has_loop: true,
            };
            if let Some(m) = measure {
                flags = flags.or(infer_exp_inner(m, it, env, scope, defined));
            }
            flags
                .or(infer_exp_inner(cond, it, env, scope, defined))
                .or(infer_exp_inner(body, it, env, scope, defined))
        }
        ExpKind::For {
            var,
            from,
            to,
            step,
            body,
            ..
        } => {
            let mut flags = EffectFlags {
                is_monadic: false,
                early_return: false,
                has_loop: true,
            };
            flags = flags
                .or(infer_exp_inner(from, it, env, scope, defined))
                .or(infer_exp_inner(to, it, env, scope, defined))
                .or(infer_exp_inner(step, it, env, scope, defined));
            let mut inner = scope.clone();
            if let Some(v) = name_text(var, it) {
                inner.insert(v.to_owned());
            }
            flags.or(infer_exp_inner(body, it, env, &inner, defined))
        }
        ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
            infer_exp_inner(a, it, env, scope, defined)
                .or(infer_exp_inner(b, it, env, scope, defined))
        }
        ExpKind::Struct { fields, .. } => {
            let mut flags = EffectFlags::PURE;
            for f in fields {
                flags = flags.or(infer_exp_inner(&f.value, it, env, scope, defined));
            }
            flags
        }
        ExpKind::StructUpdate { record, fields } => {
            let mut flags = infer_exp_inner(record, it, env, scope, defined);
            for f in fields {
                flags = flags.or(infer_exp_inner(&f.value, it, env, scope, defined));
            }
            flags
        }
        ExpKind::Field { record, .. } => infer_exp_inner(record, it, env, scope, defined),
        ExpKind::Match { scrutinee, arms } => {
            let mut flags = infer_exp_inner(scrutinee, it, env, scope, defined);
            for arm in arms {
                flags = flags.or(infer_pexp(arm, it, env, scope, defined));
            }
            flags
        }
        ExpKind::Let { pat, bind, body } => {
            let bind_flags = infer_exp_inner(bind, it, env, scope, defined);
            let mut inner = scope.clone();
            pat_binders(pat, it, &mut inner);
            bind_flags.or(infer_exp_inner(body, it, env, &inner, defined))
        }
        ExpKind::Assign { lexp, value } => infer_exp_inner(value, it, env, scope, defined)
            .or(lexp_target_flags(lexp, it, env, scope, defined)),
        ExpKind::Sizeof(_) => EffectFlags::PURE,
        // Any explicit `return` counts as early (sound over-approximation of
        // the tail-position refinement; an extra frame is always sound).
        ExpKind::Return(payload) | ExpKind::InternalReturn(payload) => {
            let mut flags = infer_exp_inner(payload, it, env, scope, defined);
            flags.early_return = true;
            flags
        }
        ExpKind::Exit(payload) | ExpKind::Throw(payload) => {
            // Mirrors sail-effects: `throw`/`exit` are impure control
            // (`function_is_pure` is false) on the `ExceptM` path.
            let mut flags = infer_exp_inner(payload, it, env, scope, defined);
            flags.is_monadic = true;
            flags.early_return = true;
            flags
        }
        ExpKind::Config(_) => EffectFlags::PURE,
        // `&reg`: register reference — a `SailM` effect like `ReadReg`.
        ExpKind::Ref(_) => EffectFlags {
            is_monadic: true,
            early_return: false,
            has_loop: false,
        },
        ExpKind::Try { exp, arms } => {
            let mut flags = infer_exp_inner(exp, it, env, scope, defined);
            for arm in arms {
                flags = flags.or(infer_pexp(arm, it, env, scope, defined));
            }
            flags
        }
        // Mirrors sail-effects: `assert` rides the `ExceptM` layer
        // (`has_early_return`) without being a `SailM` effect itself.
        ExpKind::Assert { cond, msg } => {
            let mut flags = infer_exp_inner(cond, it, env, scope, defined)
                .or(infer_exp_inner(msg, it, env, scope, defined));
            flags.early_return = true;
            flags
        }
        ExpKind::Var { lexp, init, body } => {
            let init_flags = infer_exp_inner(init, it, env, scope, defined);
            let mut inner = scope.clone();
            lexp_binders(lexp, it, &mut inner);
            // The declared names are bound at the target itself (a `var`
            // declaration is pure); index/address sub-expressions still
            // evaluate in the outer scope.
            let target_flags = lexp_target_flags_scoped(lexp, it, env, &inner, scope, defined);
            init_flags
                .or(target_flags)
                .or(infer_exp_inner(body, it, env, &inner, defined))
        }
        // `undefined`: unknown / nondeterministic value ⇒ monadic.
        ExpKind::Undef => EffectFlags {
            is_monadic: true,
            early_return: false,
            has_loop: false,
        },
        ExpKind::InternalPlet { pat, bind, body } => {
            let bind_flags = infer_exp_inner(bind, it, env, scope, defined);
            let mut inner = scope.clone();
            pat_binders(pat, it, &mut inner);
            bind_flags.or(infer_exp_inner(body, it, env, &inner, defined))
        }
        ExpKind::InternalValue(_) => EffectFlags::PURE,
        ExpKind::InternalAssume { exp, .. } => infer_exp_inner(exp, it, env, scope, defined),
        ExpKind::Constraint(_) => EffectFlags::PURE,
    }
}

fn combine_all(
    exps: &[Exp],
    it: &impl Interner,
    env: &EffectEnv,
    scope: &BTreeSet<String>,
    defined: Option<&BTreeSet<String>>,
) -> EffectFlags {
    let mut flags = EffectFlags::PURE;
    for e in exps {
        flags = flags.or(infer_exp_inner(e, it, env, scope, defined));
    }
    flags
}

/// Flags for reading a variable: bound locals are pure, everything else
/// (possible register / global, or unresolvable id) is monadic.
fn id_flags(
    name: &Name,
    it: &impl Interner,
    env: &EffectEnv,
    scope: &BTreeSet<String>,
) -> EffectFlags {
    match name_text(name, it) {
        Some(text) => {
            if scope.contains(text) {
                EffectFlags::PURE
            } else if env.is_register(text) {
                EffectFlags {
                    is_monadic: true,
                    early_return: false,
                    has_loop: false,
                }
            } else {
                // Free global: may be a register or effectful global.
                EffectFlags {
                    is_monadic: true,
                    early_return: false,
                    has_loop: false,
                }
            }
        }
        None => EffectFlags {
            is_monadic: true,
            early_return: false,
            has_loop: false,
        },
    }
}

/// Effect of writing through an `LExp` target, plus effects of any index /
/// address sub-expressions. Writes to bound locals are pure; writes through
/// `Deref` (memory) or to anything not proven local are monadic.
fn lexp_target_flags(
    lexp: &LExp,
    it: &impl Interner,
    env: &EffectEnv,
    scope: &BTreeSet<String>,
    defined: Option<&BTreeSet<String>>,
) -> EffectFlags {
    lexp_target_flags_scoped(lexp, it, env, scope, scope, defined)
}

/// [`lexp_target_flags`] with separate scopes for the target path
/// (`target_scope`: declared names are bound) and embedded index/address
/// expressions (`exp_scope`: evaluated before the declaration takes effect).
fn lexp_target_flags_scoped(
    lexp: &LExp,
    it: &impl Interner,
    env: &EffectEnv,
    target_scope: &BTreeSet<String>,
    exp_scope: &BTreeSet<String>,
    defined: Option<&BTreeSet<String>>,
) -> EffectFlags {
    match &lexp.kind {
        LExpKind::Id(name) | LExpKind::Typ { id: name, .. } => {
            id_flags(name, it, env, target_scope)
        }
        // Memory write — always a `SailM` effect.
        LExpKind::Deref(addr) => EffectFlags {
            is_monadic: true,
            early_return: false,
            has_loop: false,
        }
        .or(infer_exp_inner(addr, it, env, exp_scope, defined)),
        LExpKind::Tuple(ls) | LExpKind::VectorConcat(ls) => {
            let mut flags = EffectFlags::PURE;
            for l in ls {
                flags = flags.or(lexp_target_flags_scoped(
                    l,
                    it,
                    env,
                    target_scope,
                    exp_scope,
                    defined,
                ));
            }
            flags
        }
        LExpKind::Vector { lexp, index } => {
            lexp_target_flags_scoped(lexp, it, env, target_scope, exp_scope, defined)
                .or(infer_exp_inner(index, it, env, exp_scope, defined))
        }
        LExpKind::VectorRange { lexp, lo, hi } => {
            lexp_target_flags_scoped(lexp, it, env, target_scope, exp_scope, defined)
                .or(infer_exp_inner(lo, it, env, exp_scope, defined))
                .or(infer_exp_inner(hi, it, env, exp_scope, defined))
        }
        LExpKind::Field { lexp, .. } => {
            lexp_target_flags_scoped(lexp, it, env, target_scope, exp_scope, defined)
        }
    }
}

/// Local names bound by a value pattern.
fn pat_binders(pat: &Pat, it: &impl Interner, out: &mut BTreeSet<String>) {
    match &pat.kind {
        PatKind::Lit(_) | PatKind::Wild => {}
        PatKind::Or(a, b) | PatKind::Cons(a, b) => {
            pat_binders(a, it, out);
            pat_binders(b, it, out);
        }
        PatKind::Not(inner) => pat_binders(inner, it, out),
        PatKind::As { pat, id } => {
            pat_binders(pat, it, out);
            if let Some(text) = name_text(id, it) {
                out.insert(text.to_owned());
            }
        }
        PatKind::Typ { pat, .. } => pat_binders(pat, it, out),
        PatKind::Id(name) => {
            if let Some(text) = name_text(name, it) {
                out.insert(text.to_owned());
            }
        }
        PatKind::Var { pat, .. } => pat_binders(pat, it, out),
        PatKind::App { args, .. } => {
            for a in args {
                pat_binders(a, it, out);
            }
        }
        PatKind::Vector(pats)
        | PatKind::VectorConcat(pats)
        | PatKind::Tuple(pats)
        | PatKind::List(pats)
        | PatKind::StringAppend(pats) => {
            for p in pats {
                pat_binders(p, it, out);
            }
        }
        PatKind::VectorSubrange { .. } => {}
        PatKind::Struct { fields, .. } => {
            for (_, p) in fields {
                pat_binders(p, it, out);
            }
        }
    }
}

/// Local names declared by an `LExp` (`var` targets).
fn lexp_binders(lexp: &LExp, it: &impl Interner, out: &mut BTreeSet<String>) {
    match &lexp.kind {
        LExpKind::Id(name) | LExpKind::Typ { id: name, .. } => {
            if let Some(text) = name_text(name, it) {
                out.insert(text.to_owned());
            }
        }
        // `*p = …` declares nothing (the address is an expression).
        LExpKind::Deref(_) => {}
        LExpKind::Tuple(ls) | LExpKind::VectorConcat(ls) => {
            for l in ls {
                lexp_binders(l, it, out);
            }
        }
        LExpKind::Vector { lexp, .. } | LExpKind::Field { lexp, .. } => {
            lexp_binders(lexp, it, out);
        }
        LExpKind::VectorRange { lexp, .. } => lexp_binders(lexp, it, out),
    }
}

fn collect_calls(exp: &Exp, it: &impl Interner, out: &mut BTreeSet<String>) {
    match &exp.kind {
        ExpKind::Block(es) | ExpKind::Tuple(es) | ExpKind::Vector(es) | ExpKind::List(es) => {
            for e in es {
                collect_calls(e, it, out);
            }
        }
        ExpKind::Id(_)
        | ExpKind::Lit(_)
        | ExpKind::Sizeof(_)
        | ExpKind::Config(_)
        | ExpKind::Ref(_)
        | ExpKind::Undef
        | ExpKind::InternalValue(_)
        | ExpKind::Constraint(_) => {}
        ExpKind::Typ { exp, .. }
        | ExpKind::Return(exp)
        | ExpKind::InternalReturn(exp)
        | ExpKind::Exit(exp)
        | ExpKind::Throw(exp)
        | ExpKind::Field { record: exp, .. }
        | ExpKind::InternalAssume { exp, .. } => collect_calls(exp, it, out),
        ExpKind::App { func, args } => {
            if let Some(text) = name_text(func, it) {
                out.insert(text.to_owned());
            }
            for a in args {
                collect_calls(a, it, out);
            }
        }
        ExpKind::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_calls(cond, it, out);
            collect_calls(then_branch, it, out);
            collect_calls(else_branch, it, out);
        }
        ExpKind::Loop {
            measure,
            cond,
            body,
            ..
        } => {
            if let Some(m) = measure {
                collect_calls(m, it, out);
            }
            collect_calls(cond, it, out);
            collect_calls(body, it, out);
        }
        ExpKind::For {
            from,
            to,
            step,
            body,
            ..
        } => {
            collect_calls(from, it, out);
            collect_calls(to, it, out);
            collect_calls(step, it, out);
            collect_calls(body, it, out);
        }
        ExpKind::VectorAppend(a, b) | ExpKind::Cons(a, b) => {
            collect_calls(a, it, out);
            collect_calls(b, it, out);
        }
        ExpKind::Struct { fields, .. } => {
            for f in fields {
                collect_calls(&f.value, it, out);
            }
        }
        ExpKind::StructUpdate { record, fields } => {
            collect_calls(record, it, out);
            for f in fields {
                collect_calls(&f.value, it, out);
            }
        }
        ExpKind::Match { scrutinee, arms } => {
            collect_calls(scrutinee, it, out);
            for arm in arms {
                collect_pexp_calls(arm, it, out);
            }
        }
        ExpKind::Let { bind, body, .. } | ExpKind::InternalPlet { bind, body, .. } => {
            collect_calls(bind, it, out);
            collect_calls(body, it, out);
        }
        ExpKind::Assign { lexp, value } => {
            collect_lexp_calls(lexp, it, out);
            collect_calls(value, it, out);
        }
        ExpKind::Try { exp, arms } => {
            collect_calls(exp, it, out);
            for arm in arms {
                collect_pexp_calls(arm, it, out);
            }
        }
        ExpKind::Assert { cond, msg } => {
            collect_calls(cond, it, out);
            collect_calls(msg, it, out);
        }
        ExpKind::Var { lexp, init, body } => {
            collect_lexp_calls(lexp, it, out);
            collect_calls(init, it, out);
            collect_calls(body, it, out);
        }
    }
}

fn collect_pexp_calls(arm: &PExp, it: &impl Interner, out: &mut BTreeSet<String>) {
    match &arm.kind {
        PExpKind::PatExp { body, .. } => collect_calls(body, it, out),
        PExpKind::PatWhen { guard, body, .. } => {
            collect_calls(guard, it, out);
            collect_calls(body, it, out);
        }
    }
}

fn collect_lexp_calls(lexp: &LExp, it: &impl Interner, out: &mut BTreeSet<String>) {
    match &lexp.kind {
        LExpKind::Id(_) | LExpKind::Typ { .. } | LExpKind::Deref(_) => {
            if let LExpKind::Deref(addr) = &lexp.kind {
                collect_calls(addr, it, out);
            }
        }
        LExpKind::Tuple(ls) | LExpKind::VectorConcat(ls) => {
            for l in ls {
                collect_lexp_calls(l, it, out);
            }
        }
        LExpKind::Vector { lexp, index } => {
            collect_lexp_calls(lexp, it, out);
            collect_calls(index, it, out);
        }
        LExpKind::VectorRange { lexp, lo, hi } => {
            collect_lexp_calls(lexp, it, out);
            collect_calls(lo, it, out);
            collect_calls(hi, it, out);
        }
        LExpKind::Field { lexp, .. } => collect_lexp_calls(lexp, it, out),
    }
}

fn collect_def_calls(def: &Def, it: &impl Interner, out: &mut BTreeSet<String>) {
    match &def.kind {
        DefKind::Fundef { funcls, .. } => {
            for funcl in funcls {
                collect_pexp_calls(&funcl.arm, it, out);
            }
        }
        DefKind::Mapdef { clauses, .. } => {
            for clause in clauses {
                match &clause.kind {
                    MapclKind::Bidir { .. } => {}
                    MapclKind::Forwards(pexp) | MapclKind::Backwards(pexp) => {
                        collect_pexp_calls(pexp, it, out);
                    }
                }
            }
        }
        DefKind::Impl(funcl) => collect_pexp_calls(&funcl.arm, it, out),
        DefKind::Let { exp, .. } => collect_calls(exp, it, out),
        DefKind::Register(spec) => {
            if let Some(init) = &spec.init {
                collect_calls(init, it, out);
            }
        }
        DefKind::InternalMutrec(sets) => {
            for set in sets {
                for funcl in &set.funcls {
                    collect_pexp_calls(&funcl.arm, it, out);
                }
            }
        }
        DefKind::Outcome { defs, .. } => {
            for inner in defs {
                collect_def_calls(inner, it, out);
            }
        }
        DefKind::Scattered(scattered) => match scattered {
            sail_ast::ScatteredDef::Funcl(funcl) => collect_pexp_calls(&funcl.arm, it, out),
            sail_ast::ScatteredDef::Mapcl { clause, .. } => match &clause.kind {
                MapclKind::Bidir { .. } => {}
                MapclKind::Forwards(pexp) | MapclKind::Backwards(pexp) => {
                    collect_pexp_calls(pexp, it, out);
                }
            },
            _ => {}
        },
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{
        DefKind, ExpKind, Funcl, LExpKind, LitKind, Loop, Node, NodeId, Order, PExp, PExpKind,
        PatKind, RecOpt, SimpleInterner, Span, TAnnotOpt,
    };

    fn new_interner() -> SimpleInterner {
        SimpleInterner::new()
    }

    fn nm(it: &mut SimpleInterner, text: &str) -> Name {
        Name::dummy(it.intern(text))
    }

    fn id_exp(it: &mut SimpleInterner, text: &str) -> Exp {
        Node::dummy(ExpKind::Id(nm(it, text)))
    }

    fn num_exp(it: &mut SimpleInterner, text: &str) -> Exp {
        let sym = it.intern(text);
        Node::dummy(ExpKind::Lit(Node::dummy(LitKind::Num(sym))))
    }

    fn app_exp(it: &mut SimpleInterner, func: &str, args: Vec<Exp>) -> Exp {
        Node::dummy(ExpKind::App {
            func: nm(it, func),
            args,
        })
    }

    fn block_exp(exps: Vec<Exp>) -> Exp {
        Node::dummy(ExpKind::Block(exps))
    }

    fn pat_id(it: &mut SimpleInterner, text: &str) -> Pat {
        Node::dummy(PatKind::Id(nm(it, text)))
    }

    fn pat_wild() -> Pat {
        Node::dummy(PatKind::Wild)
    }

    fn pexp(pat: Pat, body: Exp) -> PExp {
        PExp {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            kind: PExpKind::PatExp { pat, body },
        }
    }

    fn funcl(it: &mut SimpleInterner, fname: &str, pat: Pat, body: Exp) -> Funcl {
        Funcl {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: nm(it, fname),
            arm: pexp(pat, body),
        }
    }

    fn fundef(funcls: Vec<Funcl>) -> Def {
        Node::dummy(DefKind::Fundef {
            rec: RecOpt::NonRec,
            annot: TAnnotOpt::None,
            funcls,
        })
    }

    fn loop_exp() -> Exp {
        // `while true do ()`: loop frame only, no effects.
        Node::dummy(ExpKind::Loop {
            loop_kind: Loop::While,
            measure: None,
            cond: Box::new(Node::dummy(ExpKind::Lit(Node::dummy(LitKind::True)))),
            body: Box::new(Node::dummy(ExpKind::Lit(Node::dummy(LitKind::Unit)))),
        })
    }

    #[test]
    fn flags_constants_combine() {
        assert_eq!(EffectFlags::PURE.triple(), (false, false, false));
        assert!(EffectFlags::PURE.is_pure());
        assert_eq!(EffectFlags::UNKNOWN.triple(), (true, true, true));
        assert!(!EffectFlags::UNKNOWN.is_pure());
        let loop_only = EffectFlags {
            is_monadic: false,
            early_return: false,
            has_loop: true,
        };
        let early_only = EffectFlags {
            is_monadic: false,
            early_return: true,
            has_loop: false,
        };
        assert_eq!(loop_only.or(early_only).triple(), (false, true, true));
    }

    #[test]
    fn pure_let_body_stays_pure() {
        let mut it = new_interner();
        // `let x = 1 in x`: binder is local, literal is pure.
        let body = Node::dummy(ExpKind::Let {
            pat: pat_id(&mut it, "x"),
            bind: Box::new(num_exp(&mut it, "1")),
            body: Box::new(id_exp(&mut it, "x")),
        });
        assert_eq!(
            infer_exp(&body, &it, &EffectEnv::empty()),
            EffectFlags::PURE
        );
    }

    #[test]
    fn free_id_is_monadic_unknown_is_monadic() {
        // A free variable may denote a register or global: conservative.
        let mut it = new_interner();
        assert!(infer_exp(&id_exp(&mut it, "rX"), &it, &EffectEnv::empty()).is_monadic);
        // Unresolvable symbols (no interner entry) are monadic too.
        let dangling = Node::dummy(ExpKind::Id(Name::dummy(sail_ast::Symbol::DUMMY)));
        assert!(infer_exp(&dangling, &it, &EffectEnv::empty()).is_monadic);
    }

    #[test]
    fn register_env_and_shadowing() {
        let mut it = new_interner();
        let mut env = EffectEnv::empty();
        env.registers.insert("rX".to_owned());
        // Free register read: monadic.
        let flags = infer_exp(&id_exp(&mut it, "rX"), &it, &env);
        assert!(flags.is_monadic && !flags.early_return && !flags.has_loop);
        // A local binding shadows the register name: pure again.
        let shadowed = Node::dummy(ExpKind::Let {
            pat: pat_id(&mut it, "rX"),
            bind: Box::new(num_exp(&mut it, "0")),
            body: Box::new(id_exp(&mut it, "rX")),
        });
        assert_eq!(infer_exp(&shadowed, &it, &env), EffectFlags::PURE);
    }

    #[test]
    fn ref_is_monadic() {
        let mut it = new_interner();
        let exp = Node::dummy(ExpKind::Ref(nm(&mut it, "rY")));
        let flags = infer_exp(&exp, &it, &EffectEnv::empty());
        assert!(flags.is_monadic && !flags.early_return && !flags.has_loop);
    }

    #[test]
    fn return_variants_set_early_only() {
        let mut it = new_interner();
        let expected = EffectFlags {
            is_monadic: false,
            early_return: true,
            has_loop: false,
        };
        for kind in [
            ExpKind::Return(Box::new(num_exp(&mut it, "1"))),
            ExpKind::InternalReturn(Box::new(num_exp(&mut it, "1"))),
        ] {
            assert_eq!(
                infer_exp(&Node::dummy(kind), &it, &EffectEnv::empty()),
                expected
            );
        }
        // Elaborated surface `return e` (`E_app (early_return, …)`).
        let env = EffectEnv::empty();
        let arg = num_exp(&mut it, "2");
        let exp = app_exp(&mut it, "early_return", vec![arg]);
        assert_eq!(
            infer_exp(&exp, &it, &env),
            EffectFlags {
                is_monadic: false,
                early_return: true,
                has_loop: false,
            }
        );
    }

    #[test]
    fn throw_and_exit_are_monadic_and_early() {
        // Mirrors sail-effects: `throw`/`exit` are impure (`function_is_pure`
        // is false) on the `ExceptM` path.
        let mut it = new_interner();
        let expected = EffectFlags {
            is_monadic: true,
            early_return: true,
            has_loop: false,
        };
        for kind in [
            ExpKind::Throw(Box::new(num_exp(&mut it, "0"))),
            ExpKind::Exit(Box::new(num_exp(&mut it, "0"))),
        ] {
            assert_eq!(
                infer_exp(&Node::dummy(kind), &it, &EffectEnv::empty()),
                expected
            );
        }
    }

    #[test]
    fn assert_is_early_without_sailm() {
        // Mirrors sail-effects: `assert` rides `ExceptM`, not `SailM`.
        let mut it = new_interner();
        let exp = Node::dummy(ExpKind::Assert {
            cond: Box::new(num_exp(&mut it, "1")),
            msg: Box::new(num_exp(&mut it, "0")),
        });
        assert_eq!(
            infer_exp(&exp, &it, &EffectEnv::empty()),
            EffectFlags {
                is_monadic: false,
                early_return: true,
                has_loop: false,
            }
        );
        // …but condition effects still propagate.
        let cond_ref = Node::dummy(ExpKind::Ref(nm(&mut it, "r")));
        let exp = Node::dummy(ExpKind::Assert {
            cond: Box::new(cond_ref),
            msg: Box::new(num_exp(&mut it, "0")),
        });
        assert_eq!(
            infer_exp(&exp, &it, &EffectEnv::empty()),
            EffectFlags {
                is_monadic: true,
                early_return: true,
                has_loop: false,
            }
        );
    }

    #[test]
    fn loops_set_has_loop() {
        let mut it = new_interner();
        assert_eq!(
            infer_exp(&loop_exp(), &it, &EffectEnv::empty()),
            EffectFlags {
                is_monadic: false,
                early_return: false,
                has_loop: true,
            }
        );
        // `for i = 0 to 10 step 1 do i`: index is bound in the body.
        let body = Node::dummy(ExpKind::For {
            var: nm(&mut it, "i"),
            from: Box::new(num_exp(&mut it, "0")),
            to: Box::new(num_exp(&mut it, "10")),
            step: Box::new(num_exp(&mut it, "1")),
            order: Order::Inc,
            body: Box::new(id_exp(&mut it, "i")),
        });
        assert_eq!(
            infer_exp(&body, &it, &EffectEnv::empty()),
            EffectFlags {
                is_monadic: false,
                early_return: false,
                has_loop: true,
            }
        );
    }

    #[test]
    fn undef_is_monadic() {
        let it = new_interner();
        let exp = Node::dummy(ExpKind::Undef);
        assert!(infer_exp(&exp, &it, &EffectEnv::empty()).is_monadic);
    }

    #[test]
    fn unknown_call_is_monadic_pure_call_is_pure() {
        let mut it = new_interner();
        // Unknown callee with pure args: monadic (unknown ⇒ monadic).
        let arg = num_exp(&mut it, "1");
        let call = app_exp(&mut it, "mystery_fn", vec![arg]);
        assert!(infer_exp(&call, &it, &EffectEnv::empty()).is_monadic);
        // Declared-pure callee over bound locals: pure.
        let mut env = EffectEnv::empty();
        env.pure_fns.insert("add".to_owned());
        let x_arg = id_exp(&mut it, "x");
        let call = app_exp(&mut it, "add", vec![x_arg]);
        let body = Node::dummy(ExpKind::Let {
            pat: pat_id(&mut it, "x"),
            bind: Box::new(num_exp(&mut it, "1")),
            body: Box::new(call),
        });
        assert_eq!(infer_exp(&body, &it, &env), EffectFlags::PURE);
        // …but argument effects still propagate through pure callees.
        let r_arg = Node::dummy(ExpKind::Ref(nm(&mut it, "r")));
        let body = app_exp(&mut it, "add", vec![r_arg]);
        assert!(infer_exp(&body, &it, &env).is_monadic);
    }

    #[test]
    fn local_assign_pure_global_assign_monadic() {
        let mut it = new_interner();
        // `var x = 0 in (x = 1)`: local write is pure.
        let local = Node::dummy(ExpKind::Var {
            lexp: Box::new(Node::dummy(LExpKind::Id(nm(&mut it, "x")))),
            init: Box::new(num_exp(&mut it, "0")),
            body: Box::new(Node::dummy(ExpKind::Assign {
                lexp: Box::new(Node::dummy(LExpKind::Id(nm(&mut it, "x")))),
                value: Box::new(num_exp(&mut it, "1")),
            })),
        });
        assert_eq!(
            infer_exp(&local, &it, &EffectEnv::empty()),
            EffectFlags::PURE
        );
        // `z = 1` with free `z`: possible register write ⇒ monadic.
        let global = Node::dummy(ExpKind::Assign {
            lexp: Box::new(Node::dummy(LExpKind::Id(nm(&mut it, "z")))),
            value: Box::new(num_exp(&mut it, "1")),
        });
        assert!(infer_exp(&global, &it, &EffectEnv::empty()).is_monadic);
        // `*p = v`: memory write ⇒ monadic even when `p`/`v` are bound.
        let mem = Node::dummy(ExpKind::Let {
            pat: pat_id(&mut it, "p"),
            bind: Box::new(num_exp(&mut it, "0")),
            body: Box::new(Node::dummy(ExpKind::Let {
                pat: pat_id(&mut it, "v"),
                bind: Box::new(num_exp(&mut it, "1")),
                body: Box::new(Node::dummy(ExpKind::Assign {
                    lexp: Box::new(Node::dummy(LExpKind::Deref(Box::new(id_exp(&mut it, "p"))))),
                    value: Box::new(id_exp(&mut it, "v")),
                })),
            })),
        });
        assert!(infer_exp(&mem, &it, &EffectEnv::empty()).is_monadic);
    }

    #[test]
    fn match_binds_arm_vars() {
        let mut it = new_interner();
        // `let s = 0 in match s { y -> y }`: scrutinee + arm are pure.
        let arm = pexp(pat_id(&mut it, "y"), id_exp(&mut it, "y"));
        let matched = Node::dummy(ExpKind::Match {
            scrutinee: Box::new(id_exp(&mut it, "s")),
            arms: vec![arm],
        });
        let body = Node::dummy(ExpKind::Let {
            pat: pat_id(&mut it, "s"),
            bind: Box::new(num_exp(&mut it, "0")),
            body: Box::new(matched),
        });
        assert_eq!(
            infer_exp(&body, &it, &EffectEnv::empty()),
            EffectFlags::PURE
        );
    }

    #[test]
    fn try_keeps_arm_effects() {
        let mut it = new_interner();
        let arm = pexp(pat_wild(), Node::dummy(ExpKind::Ref(nm(&mut it, "r"))));
        let body = Node::dummy(ExpKind::Try {
            exp: Box::new(num_exp(&mut it, "0")),
            arms: vec![arm],
        });
        assert!(infer_exp(&body, &it, &EffectEnv::empty()).is_monadic);
    }

    #[test]
    fn block_combines_children() {
        let mut it = new_interner();
        let body = block_exp(vec![
            num_exp(&mut it, "1"),
            Node::dummy(ExpKind::Return(Box::new(num_exp(&mut it, "2")))),
        ]);
        // Any explicit `return` counts as early, even in tail position
        // (sound over-approximation; see module docs).
        assert_eq!(
            infer_exp(&body, &it, &EffectEnv::empty()),
            EffectFlags {
                is_monadic: false,
                early_return: true,
                has_loop: false,
            }
        );
    }

    #[test]
    fn infer_def_combines_same_name_clauses() {
        let mut it = new_interner();
        let pure_body = num_exp(&mut it, "1");
        let pure_clause = funcl(&mut it, "f", pat_wild(), pure_body);
        let loop_clause = funcl(&mut it, "f", pat_wild(), loop_exp());
        let entries = infer_def(
            &fundef(vec![pure_clause, loop_clause]),
            &it,
            &EffectEnv::empty(),
        );
        assert_eq!(
            entries,
            vec![(
                "f".to_owned(),
                EffectFlags {
                    is_monadic: false,
                    early_return: false,
                    has_loop: true,
                }
            )]
        );
    }

    #[test]
    fn infer_funcl_names_and_unknown_fallback() {
        let mut it = new_interner();
        let seven = num_exp(&mut it, "7");
        let (name, flags) = infer_funcl(
            &funcl(&mut it, "pure_fn", pat_wild(), seven),
            &it,
            &EffectEnv::empty(),
        );
        assert_eq!(name, "pure_fn");
        assert_eq!(flags, EffectFlags::PURE);
        // Unresolvable function name is never dropped silently.
        let bad = Funcl {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: Name::dummy(sail_ast::Symbol::DUMMY),
            arm: pexp(pat_wild(), num_exp(&mut it, "7")),
        };
        assert_eq!(infer_funcl(&bad, &it, &EffectEnv::empty()).0, "<unknown>");
    }

    #[test]
    fn non_executable_defs_yield_no_entries() {
        let it = new_interner();
        let pragma = Node::dummy(DefKind::Pragma {
            key: "k".to_owned(),
            payload: "p".to_owned(),
        });
        assert!(infer_def(&pragma, &it, &EffectEnv::empty()).is_empty());
    }

    #[test]
    fn let_def_names_first_binder() {
        let mut it = new_interner();
        let def = Node::dummy(DefKind::Let {
            pat: pat_id(&mut it, "answer"),
            exp: num_exp(&mut it, "42"),
        });
        assert_eq!(
            infer_def(&def, &it, &EffectEnv::empty()),
            vec![("answer".to_owned(), EffectFlags::PURE)]
        );
    }

    #[test]
    fn from_ir_matches_sail_effects_predicates() {
        use sail_effects::{Block as IBlock, Expr as IExpr, FunDef as IFunDef, Stmt as IStmt};
        // Pure body: `(false, false, false)`.
        let pure = IFunDef::new(
            "add",
            IBlock::new(vec![IStmt::Return(IExpr::Var("x".to_owned()))]),
        );
        assert_eq!(EffectFlags::from_ir(&pure), EffectFlags::PURE);
        // Register write: monadic, no early return, no loop.
        let writer = IFunDef::new(
            "set_x",
            IBlock::new(vec![IStmt::WriteReg {
                reg: "x".to_owned(),
                expr: IExpr::Lit(1),
            }]),
        );
        assert_eq!(
            EffectFlags::from_ir(&writer),
            EffectFlags {
                is_monadic: true,
                early_return: false,
                has_loop: false,
            }
        );
        // `throw`: impure on the `ExceptM` path (mirrors `function_is_pure`).
        let thrower = IFunDef::new("t", IBlock::new(vec![IStmt::Throw(IExpr::Lit(0))]));
        assert_eq!(
            EffectFlags::from_ir(&thrower),
            EffectFlags {
                is_monadic: true,
                early_return: true,
                has_loop: false,
            }
        );
    }

    #[test]
    fn from_label_needs_independent_predicates() {
        use sail_effects::EffectLabel;
        assert_eq!(
            EffectFlags::from_label(EffectLabel::Pure, false, false),
            EffectFlags::PURE
        );
        // The coarse label cannot spell pure-with-early-return on its own;
        // the independent predicate carries it (cf. `FunSig`).
        assert_eq!(
            EffectFlags::from_label(EffectLabel::Pure, true, false),
            EffectFlags {
                is_monadic: false,
                early_return: true,
                has_loop: false,
            }
        );
        assert_eq!(
            EffectFlags::from_label(EffectLabel::SailM, false, true),
            EffectFlags {
                is_monadic: true,
                early_return: false,
                has_loop: true,
            }
        );
        assert!(
            EffectFlags::from_label(EffectLabel::ExceptM, true, false).is_monadic,
            "ExceptM (throw) is impure, like `function_is_pure`"
        );
    }

    #[test]
    fn program_propagates_monadic_transitively() {
        let mut it = new_interner();
        // `g` reads a register (monadic); `f` calls `g`; `h` is pure;
        // `c` calls looping-but-pure `l` (no flag propagation for loops).
        let g_body = Node::dummy(ExpKind::Ref(nm(&mut it, "r")));
        let def_g = fundef(vec![funcl(&mut it, "g", pat_wild(), g_body)]);
        let f_body = app_exp(&mut it, "g", vec![]);
        let def_f = fundef(vec![funcl(&mut it, "f", pat_wild(), f_body)]);
        let h_body = num_exp(&mut it, "1");
        let def_h = fundef(vec![funcl(&mut it, "h", pat_wild(), h_body)]);
        let def_l = fundef(vec![funcl(&mut it, "l", pat_wild(), loop_exp())]);
        let c_body = app_exp(&mut it, "l", vec![]);
        let def_c = fundef(vec![funcl(&mut it, "c", pat_wild(), c_body)]);
        let map = infer_program(
            &[def_g, def_f, def_h, def_l, def_c],
            &it,
            &EffectEnv::empty(),
        );
        assert!(map["g"].is_monadic, "register read");
        assert!(map["f"].is_monadic, "transitive via call to g");
        assert!(
            !map["f"].early_return && !map["f"].has_loop,
            "only is_monadic propagates"
        );
        assert_eq!(map["h"], EffectFlags::PURE);
        assert!(!map["l"].is_monadic && map["l"].has_loop);
        assert_eq!(
            map["c"],
            EffectFlags::PURE,
            "calls do not propagate has_loop"
        );
    }

    #[test]
    fn program_unknown_callee_stays_monadic() {
        let mut it = new_interner();
        let missing = app_exp(&mut it, "not_defined_anywhere", vec![]);
        let def_f = fundef(vec![funcl(&mut it, "f", pat_wild(), missing)]);
        let map = infer_program(&[def_f], &it, &EffectEnv::empty());
        assert!(map["f"].is_monadic, "unknown ⇒ monadic");
    }

    #[test]
    fn callees_of_exp_collects_app_names() {
        let mut it = new_interner();
        let one = num_exp(&mut it, "1");
        let g1 = app_exp(&mut it, "g", vec![one]);
        let h0 = app_exp(&mut it, "h", vec![]);
        let g2 = app_exp(&mut it, "g", vec![h0]);
        let exp = block_exp(vec![g1, g2]);
        let calls = callees_of_exp(&exp, &it);
        assert_eq!(calls, BTreeSet::from(["g".to_owned(), "h".to_owned()]));
    }
}
