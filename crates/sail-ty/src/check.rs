#![forbid(unsafe_code)]
//! Bidirectional checking (`infer` / `check`) + register-type checking.
//!
//! All entry points take [`CheckLimits`] fuel and terminate with
//! [`TypeErrorKind::FuelExhausted`](crate::error::TypeErrorKind) instead of
//! looping when the [`Solver`](crate::solver::Solver) answers `Unknown`.

use crate::env::TypeEnv;
use crate::error::{Span, TypeError, TypeErrorKind, TypeResult};
use crate::kind::{KindEnv, check_constraint_kind, check_typ_kind};
use crate::nexp::{MiniExp, MiniTyp, NConstraint, Nexp, TypQuant};
use crate::pat::MiniPat;
use crate::prove::{SolverContext, kenv_of_quants, prove_in_context};
use crate::solver::{ProveResult, Solver};
use crate::subsume::{check_subtype, freshen_scheme, instantiate_scheme};

/// Fuel / depth limits. Defaults are generous for the `test/lean` subset
/// while still guaranteeing termination without a solver.
#[derive(Clone, Copy, Debug)]
pub struct CheckLimits {
    /// Max recursive checker steps per entry point.
    pub max_steps: u64,
    /// Max accepted `Nexp`/`NConstraint` tree depth.
    pub max_depth: usize,
}

impl Default for CheckLimits {
    fn default() -> Self {
        Self {
            max_steps: 10_000,
            max_depth: 256,
        }
    }
}

/// Namespace of a `scattered` definition block (OCaml `parse_ast`
/// `scattered_def_aux`: `SD_function` / `SD_mapping` / `SD_variant` (plus
/// `SD_unioncl`) / `SD_enum`, each closed by its own `SD_end`).
/// Functions, mappings, unions and enums live in separate namespaces — a
/// `function f` and a `mapping f` may coexist — so clause summaries carry
/// the namespace; the old bare `{name}#{i}` names collided across kinds at
/// `end`-linkage (same string for `function f#0` and `mapping f#0`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScatteredKind {
    Function,
    Mapping,
    Union,
    Enum,
}

impl ScatteredKind {
    #[must_use]
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Mapping => "mapping",
            Self::Union => "union",
            Self::Enum => "enum",
        }
    }
}

/// Summary emitted per checked top-level definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypedDefSummary {
    pub name: String,
    pub scheme: TypQuant,
}

/// Bidirectional checker parameterized over a [`Solver`].
pub struct BidirectionalChecker<'s, S: Solver> {
    solver: &'s S,
    limits: CheckLimits,
}

impl<'s, S: Solver> BidirectionalChecker<'s, S> {
    #[must_use]
    pub fn new(solver: &'s S, limits: CheckLimits) -> Self {
        Self { solver, limits }
    }

    /// `infer(e)`: synthesize a type for `e`.
    pub fn infer(&self, exp: &MiniExp, env: &TypeEnv, span: Span) -> TypeResult<MiniTyp> {
        self.infer_fuel(exp, env, span, &mut self.limits.max_steps)
    }

    /// `check(e <= t)`: verify `e` against a known type, discharging any
    /// generated numeric obligation via the solver.
    pub fn check(
        &self,
        exp: &MiniExp,
        expected: &MiniTyp,
        env: &TypeEnv,
        span: Span,
    ) -> TypeResult<()> {
        self.check_fuel(exp, expected, env, span, &mut self.limits.max_steps)
    }

    /// Check a quantified scheme: kinds are well-formed, bound vars cover
    /// the free vars of constraint and body (no escaping unification vars
    /// at this skeleton layer).
    pub fn check_scheme(&self, name: &str, scheme: &TypQuant, span: Span) -> TypeResult<()> {
        let mut kenv = KindEnv::new();
        for (v, k) in &scheme.quants {
            kenv.bind(v.clone(), *k);
        }
        check_constraint_kind(&scheme.constraint, &kenv, span)
            .map_err(|e| self.ctx(name, span, e))?;
        check_typ_kind(&scheme.typ, &kenv, span).map_err(|e| self.ctx(name, span, e))?;
        // Every free var must be bound.
        let mut free = Vec::new();
        scheme.constraint.free_vars(&mut free);
        let mut body_nexps = Vec::new();
        scheme.typ.nexps(&mut body_nexps);
        for n in &body_nexps {
            n.free_vars(&mut free);
        }
        for v in free {
            if !scheme.quants.iter().any(|(q, _)| q == &v) {
                return Err(TypeError {
                    span,
                    kind: TypeErrorKind::QuantError(format!(
                        "def '{name}': free level var '{v}' not bound by forall"
                    )),
                });
            }
        }
        // Depth bound => termination even for adversarial inputs.
        if scheme.constraint.depth() > self.limits.max_depth {
            return Err(TypeError::fuel_exhausted(span));
        }
        Ok(())
    }

    /// Check a register declaration: its type must be well-kinded and closed
    /// under its quantifier telescope; returns a summary for the oracle diff.
    pub fn check_register(
        &self,
        name: &str,
        scheme: &TypQuant,
        span: Span,
    ) -> TypeResult<TypedDefSummary> {
        self.check_register_in_context(name, scheme, &SolverContext::new(), span)
    }

    /// Check a register declaration under a solver context (Lane 107):
    /// kind closure exactly like [`Self::check_register`], plus the scheme
    /// precondition is discharged via [`prove_in_context`] — `Refuted`
    /// preconditions are hard errors (wrapped as `RegisterError`),
    /// `Proved`/`Unknown` keep the scheme (mirrors the `check_val_def`
    /// precondition policy; sound on `Unknown`).
    pub fn check_register_in_context(
        &self,
        name: &str,
        scheme: &TypQuant,
        ctx: &SolverContext,
        span: Span,
    ) -> TypeResult<TypedDefSummary> {
        self.check_scheme(name, scheme, span).map_err(|e| TypeError {
            span: e.span,
            kind: TypeErrorKind::RegisterError(format!("register '{name}': {:?}", e.kind)),
        })?;
        let kenv = kenv_of_quants(&scheme.quants);
        match prove_in_context(self.solver, &kenv, ctx, &scheme.constraint, span) {
            ProveResult::Proved | ProveResult::Unknown => {}
            ProveResult::Refuted => {
                return Err(TypeError {
                    span,
                    kind: TypeErrorKind::RegisterError(format!(
                        "register '{name}': {}",
                        disp(&TypeError::constraint_not_proved(span, &scheme.constraint))
                    )),
                });
            }
        }
        Ok(TypedDefSummary {
            name: name.to_string(),
            scheme: scheme.clone(),
        })
    }

    /// Check a `val` spec + body: `body <= scheme.typ` under monomorphic
    /// locals extended from the (closed) scheme.
    pub fn check_val_def(
        &self,
        name: &str,
        scheme: &TypQuant,
        body: &MiniExp,
        env: &TypeEnv,
        span: Span,
    ) -> TypeResult<TypedDefSummary> {
        self.check_val_def_in_context(name, scheme, body, env, &SolverContext::new(), span)
    }

    /// Check a `val` spec + body under a solver context (Lane 107): same
    /// policy as [`Self::check_val_def`], but the scheme precondition is
    /// discharged via [`prove_in_context`] — the context carries the
    /// `val`/`register`/`scattered` assumptions accumulated so far, all
    /// through the [`Solver`] trait (Z3 only behind it, Lane 06/80).
    pub fn check_val_def_in_context(
        &self,
        name: &str,
        scheme: &TypQuant,
        body: &MiniExp,
        env: &TypeEnv,
        ctx: &SolverContext,
        span: Span,
    ) -> TypeResult<TypedDefSummary> {
        self.check_scheme(name, scheme, span)?;
        // Polymorphic schemes are skolemized (OCaml `freshen_bind` at use
        // sites): freshen the telescope with gensym names, discharge the
        // freshened precondition, then check the body against the freshened
        // body type. The old code returned here without ever checking the
        // body, so any body was accepted against any polymorphic scheme.
        // (Full instantiation stays Lane 09; `Unknown` preconditions keep
        // the abstract body per the definition-site policy, `Refuted` is a
        // hard error.)
        if !scheme.quants.is_empty() {
            let fresh = freshen_scheme(scheme, "sk");
            let kenv = kenv_of_quants(&fresh.quants);
            match prove_in_context(self.solver, &kenv, ctx, &fresh.constraint, span) {
                ProveResult::Proved | ProveResult::Unknown => {}
                ProveResult::Refuted => {
                    return Err(TypeError::constraint_not_proved(span, &fresh.constraint));
                }
            }
            self.check(body, &fresh.typ, env, span)?;
            return Ok(TypedDefSummary {
                name: name.to_string(),
                scheme: scheme.clone(),
            });
        }
        // Discharge the scheme constraint under the solver context;
        // `Unknown` is not fatal here (kept as `Unknown`, body still
        // checked) — but a `Refuted` precondition is a hard error.
        let kenv = kenv_of_quants(&scheme.quants);
        match prove_in_context(self.solver, &kenv, ctx, &scheme.constraint, span) {
            ProveResult::Proved | ProveResult::Unknown => {}
            ProveResult::Refuted => {
                return Err(TypeError::constraint_not_proved(span, &scheme.constraint));
            }
        }
        self.check(body, &scheme.typ, env, span)?;
        Ok(TypedDefSummary {
            name: name.to_string(),
            scheme: scheme.clone(),
        })
    }

    /// Check `scattered` clauses of one namespace under an accumulated
    /// solver context (Lane 107): every clause scheme must be closed
    /// ([`Self::check_scheme`]); clause `i`'s precondition is discharged
    /// via [`prove_in_context`] under the constraints of `clauses[..i]`
    /// (same-telescope accumulation, Sail elaboration order) — `Refuted`
    /// is a hard error, `Proved`/`Unknown` continue soundly. Returns one
    /// summary per clause (`"<kind> <name>#<i>"`, namespaced so `end`
    /// linkage cannot confuse kinds sharing a name).
    pub fn check_scattered(
        &self,
        kind: ScatteredKind,
        name: &str,
        clauses: &[TypQuant],
        span: Span,
    ) -> TypeResult<Vec<TypedDefSummary>> {
        let mut ctx = SolverContext::new();
        let mut out = Vec::with_capacity(clauses.len());
        for (i, scheme) in clauses.iter().enumerate() {
            let clause_name = format!("{} {name}#{i}", kind.prefix());
            self.check_scheme(&clause_name, scheme, span)?;
            let kenv = kenv_of_quants(&scheme.quants);
            match prove_in_context(self.solver, &kenv, &ctx, &scheme.constraint, span) {
                ProveResult::Proved | ProveResult::Unknown => {}
                ProveResult::Refuted => {
                    return Err(TypeError::constraint_not_proved(span, &scheme.constraint));
                }
            }
            ctx.assume(scheme.constraint.clone());
            out.push(TypedDefSummary {
                name: clause_name,
                scheme: scheme.clone(),
            });
        }
        Ok(out)
    }

    /// Check a value pattern against a scrutinee type (rules `P-*`).
    ///
    /// Returns the bindings in left-to-right order. Fuel-guarded like the
    /// expression entry points.
    pub fn check_pat(
        &self,
        pat: &MiniPat,
        expected: &MiniTyp,
        span: Span,
    ) -> TypeResult<Vec<(String, MiniTyp)>> {
        self.check_pat_fuel(pat, expected, span, &mut self.limits.max_steps)
    }

    // -- internals -------------------------------------------------------

    fn ctx(&self, name: &str, span: Span, e: TypeError) -> TypeError {
        TypeError {
            span,
            kind: TypeErrorKind::QuantError(format!("def '{name}': {}", disp(&e))),
        }
    }

    fn take_fuel(&self, fuel: &mut u64, span: Span) -> TypeResult<()> {
        if *fuel == 0 {
            return Err(TypeError::fuel_exhausted(span));
        }
        *fuel -= 1;
        Ok(())
    }

    fn infer_fuel(
        &self,
        exp: &MiniExp,
        env: &TypeEnv,
        span: Span,
        fuel: &mut u64,
    ) -> TypeResult<MiniTyp> {
        // Fuel is threaded by mutable reference so every sibling
        // subcomputation consumes from one shared budget (by-value copies
        // silently reset the budget per branch).
        self.take_fuel(fuel, span)?;
        match exp {
            MiniExp::IntLit(_) => Ok(MiniTyp::Int),
            MiniExp::BoolLit(_) => Ok(MiniTyp::Bool),
            MiniExp::Var(v) => {
                // Fast path: locals + monomorphic schemes (rules `E-*`).
                if let Some(t) = env.lookup(v) {
                    return Ok(t);
                }
                // Polymorphic instantiation (rules `Q-Inst`, `I-VarPoly`):
                // freshen the scheme, discharge the precondition, yield the
                // body type. Only `Proved` yields the type: `infer` returns
                // just the type, so an `Unknown` obligation would have no
                // channel to travel on and be silently dropped (OCaml keeps
                // it in the env constraint set instead). `Refuted` and
                // `Unknown` are hard errors here (rule `Z-UnknownSound`).
                // Monomorphic behaviour above is unchanged.
                let scheme = env
                    .lookup_val(v)
                    .or_else(|| env.lookup_register(v))
                    .cloned();
                match scheme {
                    Some(q) => {
                        let (obl, typ) = instantiate_scheme(&q);
                        match self.solver.prove(&obl) {
                            ProveResult::Proved => Ok(typ),
                            ProveResult::Refuted | ProveResult::Unknown => {
                                Err(TypeError::constraint_not_proved(span, &obl))
                            }
                        }
                    }
                    None => Err(TypeError::unbound_var(span, v.clone())),
                }
            }
            MiniExp::Annot(e, t) => {
                self.check_fuel(e, t, env, span, &mut *fuel)?;
                Ok(t.clone())
            }
            MiniExp::Tuple(es) => {
                let mut ts = Vec::with_capacity(es.len());
                for e in es {
                    self.take_fuel(&mut *fuel, span)?;
                    ts.push(self.infer_fuel(e, env, span, &mut *fuel)?);
                }
                Ok(MiniTyp::Tuple(ts))
            }
            MiniExp::If(c, a, b) => {
                self.check_fuel(c, &MiniTyp::Bool, env, span, &mut *fuel)?;
                let ta = self.infer_fuel(a, env, span, &mut *fuel)?;
                self.check_fuel(b, &ta, env, span, &mut *fuel)?;
                Ok(ta)
            }
            MiniExp::Let { var, annot, bound, body } => {
                let tb = self.infer_fuel(bound, env, span, &mut *fuel)?;
                let ascribed = annot.clone().unwrap_or(tb);
                // `let x: t = v`: check bound value against ascription.
                self.check_fuel(bound, &ascribed, env, span, &mut *fuel)?;
                let mut inner = env.clone();
                inner.add_local(var.clone(), ascribed);
                self.infer_fuel(body, &inner, span, &mut *fuel)
            }
            MiniExp::App(f, x) => {
                let tf = self.infer_fuel(f, env, span, &mut *fuel)?;
                match tf {
                    MiniTyp::Fun(dom, cod) => {
                        self.check_fuel(x, &dom, env, span, &mut *fuel)?;
                        Ok((*cod).clone())
                    }
                    other => Err(TypeError::mismatch(
                        span,
                        "(? -> ?)".to_string(),
                        other.to_string(),
                    )),
                }
            }
            // Lane 105: representable-but-unchecked extended forms
            // (`Block`/`Loop`/`For`/`Match`/… — every `E_*` outside the
            // Lane-79 subset). No inference rules added or changed; these
            // report a mismatch so they can never silently succeed.
            _ => Err(TypeError::mismatch(
                span,
                "lane-79 inferable expression".to_string(),
                "extended MiniExp form (representable, checking deferred)".to_string(),
            )),
        }
    }

    #[allow(clippy::only_used_in_recursion)]
    fn check_fuel(
        &self,
        exp: &MiniExp,
        expected: &MiniTyp,
        env: &TypeEnv,
        span: Span,
        fuel: &mut u64,
    ) -> TypeResult<()> {
        self.take_fuel(fuel, span)?;
        // Depth guard on the expected type's level terms.
        let mut ns = Vec::new();
        expected.nexps(&mut ns);
        for n in &ns {
            if n.depth() > self.limits.max_depth {
                return Err(TypeError::fuel_exhausted(span));
            }
        }
        match (exp, expected) {
            // Int literal against `int` or a singleton `range(lo,hi)`: if the
            // bounds are constant, decide syntactically; else ask the solver
            // for `lo <= c <= hi` and accept `Unknown` only via annotation
            // fallback below (here: hard error to stay sound).
            (MiniExp::IntLit(c), MiniTyp::Range(lo, hi)) => {
                match (lo.as_const(), hi.as_const()) {
                    (Some(l), Some(h)) => {
                        if *c >= l && *c <= h {
                            Ok(())
                        } else {
                            Err(TypeError::mismatch(
                                span,
                                expected.to_string(),
                                format!("literal {c}"),
                            ))
                        }
                    }
                    _ => {
                        let nc = NConstraint::And(
                            Box::new(NConstraint::Le(lo.clone(), Box::new(Nexp::Const(*c)))),
                            Box::new(NConstraint::Le(
                                Box::new(Nexp::Const(*c)),
                                hi.clone(),
                            )),
                        );
                        match self.solver.prove(&nc) {
                            ProveResult::Proved => Ok(()),
                            ProveResult::Refuted | ProveResult::Unknown => {
                                Err(TypeError::constraint_not_proved(span, &nc))
                            }
                        }
                    }
                }
            }
            // `bits('n)` length obligations: any literal is fine (length is a
            // type-level term, not the value); kind check only.
            (_, MiniTyp::Bits(n)) => {
                if n.depth() > self.limits.max_depth {
                    return Err(TypeError::fuel_exhausted(span));
                }
                let inferred = self.infer_fuel(exp, env, span, &mut *fuel)?;
                self.unify(&inferred, expected, span)
            }
            _ => {
                let inferred = self.infer_fuel(exp, env, span, &mut *fuel)?;
                self.unify(&inferred, expected, span)
            }
        }
    }

    /// Structural subsumption (Sail `subtyp`, rules `S-*`/`X-*`): equal
    /// constructors unify; `range <: int` holds; `bits` lengths and `range`
    /// inclusions delegate to the solver (sound on `Unknown`, see
    /// [`check_subtype`]).
    fn unify(&self, inferred: &MiniTyp, expected: &MiniTyp, span: Span) -> TypeResult<()> {
        if inferred == expected {
            // Discharge well-kindedness of level terms with the sorted
            // opaque kind env (int- vs bool-level vars distinguished,
            // kind-confused names left unbound and reported).
            // (Full quant context comes from check_scheme.)
            let kenv = opaque_kenv(inferred);
            check_typ_kind(expected, &kenv, span)?;
            return Ok(());
        }
        let _ = check_subtype(self.solver, inferred, expected, span)?;
        // Well-kindedness of the expected skeleton (sorted opaque env).
        let kenv = opaque_kenv(expected);
        check_typ_kind(expected, &kenv, span)?;
        Ok(())
    }

    fn check_pat_fuel(
        &self,
        pat: &MiniPat,
        expected: &MiniTyp,
        span: Span,
        fuel: &mut u64,
    ) -> TypeResult<Vec<(String, MiniTyp)>> {
        self.take_fuel(fuel, span)?;
        match pat {
            // `P-Wild`: binds nothing.
            MiniPat::Wild => Ok(Vec::new()),
            // `P-Var`: binds the scrutinee type.
            MiniPat::Var(v) => Ok(vec![(v.clone(), expected.clone())]),
            // `P-Lit`: the literal must check against the scrutinee type.
            MiniPat::IntLit(c) => {
                let empty = TypeEnv::new();
                self.check_fuel(&MiniExp::IntLit(*c), expected, &empty, span, &mut *fuel)?;
                Ok(Vec::new())
            }
            MiniPat::BoolLit(b) => {
                let empty = TypeEnv::new();
                self.check_fuel(&MiniExp::BoolLit(*b), expected, &empty, span, &mut *fuel)?;
                Ok(Vec::new())
            }
            // `P-Tuple`: pointwise against a same-arity tuple type.
            MiniPat::Tuple(ps) => match expected {
                MiniTyp::Tuple(ts) => {
                    if ps.len() != ts.len() {
                        return Err(TypeError {
                            span,
                            kind: TypeErrorKind::ArityMismatch {
                                expected: ts.len(),
                                found: ps.len(),
                            },
                        });
                    }
                    let mut out = Vec::new();
                    for (p, t) in ps.iter().zip(ts.iter()) {
                        self.take_fuel(&mut *fuel, span)?;
                        out.extend(self.check_pat_fuel(p, t, span, &mut *fuel)?);
                    }
                    Ok(out)
                }
                _ => Err(TypeError::mismatch(
                    span,
                    expected.to_string(),
                    "tuple pattern".to_string(),
                )),
            },
            // `P-Annot`: check inside against the annotation, then require
            // the annotation to be a subtype of the scrutinee type.
            MiniPat::Annot(inner, annot) => {
                let bindings = self.check_pat_fuel(inner, annot, span, &mut *fuel)?;
                let _ = check_subtype(self.solver, annot, expected, span)?;
                Ok(bindings)
            },
        }
    }
}

fn opaque_kenv(typ: &MiniTyp) -> KindEnv {
    // Level-variable sorts (OCaml kind-annotated kids): `Nexp::Var`
    // occurrences are `K_int`-like, `NConstraint::Var` occurrences are
    // `K_bool` — one flat namespace, two kinds. The old code bound EVERY free
    // variable as `K_int`, which masked kind confusion (a `K_bool` variable
    // in an `Nexp::Var` position passed silently) and could equally reject
    // well-kinded `if 'b …` conditions. Bind each sort precisely; a name
    // used in both positions is genuinely ill-kinded and stays unbound so
    // the kind check reports it instead of guessing.
    let mut kenv = KindEnv::new();
    let mut ns = Vec::new();
    typ.nexps(&mut ns);
    let mut ints = Vec::new();
    let mut bools = Vec::new();
    for n in &ns {
        sort_level_vars(n, &mut ints, &mut bools);
    }
    for v in &ints {
        if !bools.iter().any(|b| b == v) && kenv.lookup(v).is_none() {
            kenv.bind(v.as_str(), crate::kind::Kind::KInt);
        }
    }
    for v in &bools {
        if !ints.iter().any(|i| i == v) && kenv.lookup(v).is_none() {
            kenv.bind(v.as_str(), crate::kind::Kind::KBool);
        }
    }
    kenv
}

/// Split the free variables of a level term into int-level (`Nexp::Var`,
/// including nexps nested inside constraint positions) and bool-level
/// (`NConstraint::Var`) occurrences.
fn sort_level_vars(n: &Nexp, ints: &mut Vec<String>, bools: &mut Vec<String>) {
    match n {
        Nexp::Const(_) => {}
        Nexp::Var(v) => ints.push(v.clone()),
        Nexp::Neg(a) | Nexp::Abs(a) | Nexp::Exp(a) => sort_level_vars(a, ints, bools),
        Nexp::Add(a, b)
        | Nexp::Sub(a, b)
        | Nexp::Mul(a, b)
        | Nexp::Div(a, b)
        | Nexp::Mod(a, b) => {
            sort_level_vars(a, ints, bools);
            sort_level_vars(b, ints, bools);
        }
        Nexp::If(c, a, b) => {
            sort_constraint_vars(c, ints, bools);
            sort_level_vars(a, ints, bools);
            sort_level_vars(b, ints, bools);
        }
    }
}

fn sort_constraint_vars(c: &NConstraint, ints: &mut Vec<String>, bools: &mut Vec<String>) {
    match c {
        NConstraint::True | NConstraint::False | NConstraint::Id(_) => {}
        NConstraint::Var(v) => bools.push(v.clone()),
        NConstraint::Eq(a, b)
        | NConstraint::Neq(a, b)
        | NConstraint::Ge(a, b)
        | NConstraint::Gt(a, b)
        | NConstraint::Le(a, b)
        | NConstraint::Lt(a, b) => {
            sort_level_vars(a, ints, bools);
            sort_level_vars(b, ints, bools);
        }
        NConstraint::And(a, b) | NConstraint::Or(a, b) => {
            sort_constraint_vars(a, ints, bools);
            sort_constraint_vars(b, ints, bools);
        }
        NConstraint::Not(a) => sort_constraint_vars(a, ints, bools),
        NConstraint::Set(n, _) => sort_level_vars(n, ints, bools),
        NConstraint::App(_, args) => {
            for a in args {
                sort_level_vars(a, ints, bools);
            }
        }
    }
}

fn disp(e: &TypeError) -> String {
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solver::StubSolver;

    #[test]
    fn opaque_kenv_sorts_int_and_bool_level_vars() {
        // `Nexp::Var` occurrences are `K_int`, `NConstraint::Var`
        // occurrences are `K_bool` (one flat namespace, two kinds — the
        // skeleton documents this in `NConstraint::free_vars`). The old
        // code bound everything as `K_int`, masking kind confusion; a name
        // used in both positions stays unbound so the kind check reports
        // it instead of guessing.
        let int_only = MiniTyp::Bits(Box::new(Nexp::Var("n".to_string())));
        assert_eq!(
            opaque_kenv(&int_only).lookup("n"),
            Some(crate::kind::Kind::KInt)
        );
        let bool_only = MiniTyp::Bits(Box::new(Nexp::If(
            Box::new(NConstraint::Var("b".to_string())),
            Box::new(Nexp::Const(1)),
            Box::new(Nexp::Const(2)),
        )));
        assert_eq!(
            opaque_kenv(&bool_only).lookup("b"),
            Some(crate::kind::Kind::KBool)
        );
        let confused = MiniTyp::Bits(Box::new(Nexp::If(
            Box::new(NConstraint::Var("x".to_string())),
            Box::new(Nexp::Var("x".to_string())),
            Box::new(Nexp::Const(0)),
        )));
        assert_eq!(opaque_kenv(&confused).lookup("x"), None);
    }

    #[test]
    fn fuel_budget_is_shared_across_siblings() {
        // By-`&mut` threading every sibling consumes from one budget, so a
        // wide tuple exhausts a small budget. By-value copies silently
        // reset the budget per branch and accepted this (the documented
        // `max_steps` contract is per entry point, not per branch).
        let s = StubSolver::new();
        let c = BidirectionalChecker::new(
            &s,
            CheckLimits { max_steps: 10, max_depth: 256 },
        );
        let env = TypeEnv::new();
        let wide = MiniExp::Tuple(vec![MiniExp::IntLit(0); 8]);
        assert!(c.infer(&wide, &env, Span::dummy()).is_err());
        let narrow = MiniExp::Tuple(vec![MiniExp::IntLit(0); 2]);
        assert!(c.infer(&narrow, &env, Span::dummy()).is_ok());
    }
}
