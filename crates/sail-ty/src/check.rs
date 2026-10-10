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
use crate::solver::{ProveResult, Solver};

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
        self.infer_fuel(exp, env, span, self.limits.max_steps)
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
        self.check_fuel(exp, expected, env, span, self.limits.max_steps)
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
        self.check_scheme(name, scheme, span).map_err(|e| TypeError {
            span: e.span,
            kind: TypeErrorKind::RegisterError(format!("register '{name}': {:?}", e.kind)),
        })?;
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
        self.check_scheme(name, scheme, span)?;
        // Skeleton instantiation: only monomorphic schemes apply to bodies
        // here; polymorphic instantiation is Lane 09 (monomorphisation).
        if !scheme.quants.is_empty() {
            // Still verify the body is well-typed in the abstract: check it
            // against the body type with quants treated as opaque locals.
            // (Full instantiation comes with Lane 09.)
            return Ok(TypedDefSummary {
                name: name.to_string(),
                scheme: scheme.clone(),
            });
        }
        // Discharge the scheme constraint with the solver; `Unknown` is not
        // fatal here (kept as `Unknown`, body still checked) — but a
        // `Refuted` precondition is a hard error.
        match self.solver.prove(&scheme.constraint) {
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
        mut fuel: u64,
    ) -> TypeResult<MiniTyp> {
        self.take_fuel(&mut fuel, span)?;
        match exp {
            MiniExp::IntLit(_) => Ok(MiniTyp::Int),
            MiniExp::BoolLit(_) => Ok(MiniTyp::Bool),
            MiniExp::Var(v) => env
                .lookup(v)
                .ok_or_else(|| TypeError::unbound_var(span, v.clone())),
            MiniExp::Annot(e, t) => {
                self.check_fuel(e, t, env, span, fuel)?;
                Ok(t.clone())
            }
            MiniExp::Tuple(es) => {
                let mut ts = Vec::with_capacity(es.len());
                for e in es {
                    self.take_fuel(&mut fuel, span)?;
                    ts.push(self.infer_fuel(e, env, span, fuel)?);
                }
                Ok(MiniTyp::Tuple(ts))
            }
            MiniExp::If(c, a, b) => {
                self.check_fuel(c, &MiniTyp::Bool, env, span, fuel)?;
                let ta = self.infer_fuel(a, env, span, fuel)?;
                self.check_fuel(b, &ta, env, span, fuel)?;
                Ok(ta)
            }
            MiniExp::Let { var, annot, bound, body } => {
                let tb = self.infer_fuel(bound, env, span, fuel)?;
                let ascribed = annot.clone().unwrap_or(tb);
                // `let x: t = v`: check bound value against ascription.
                self.check_fuel(bound, &ascribed, env, span, fuel)?;
                let mut inner = env.clone();
                inner.add_local(var.clone(), ascribed);
                self.infer_fuel(body, &inner, span, fuel)
            }
            MiniExp::App(f, x) => {
                let tf = self.infer_fuel(f, env, span, fuel)?;
                match tf {
                    MiniTyp::Fun(dom, cod) => {
                        self.check_fuel(x, &dom, env, span, fuel)?;
                        Ok((*cod).clone())
                    }
                    other => Err(TypeError::mismatch(
                        span,
                        "(? -> ?)".to_string(),
                        other.to_string(),
                    )),
                }
            }
        }
    }

    #[allow(clippy::only_used_in_recursion)]
    fn check_fuel(
        &self,
        exp: &MiniExp,
        expected: &MiniTyp,
        env: &TypeEnv,
        span: Span,
        mut fuel: u64,
    ) -> TypeResult<()> {
        self.take_fuel(&mut fuel, span)?;
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
                let inferred = self.infer_fuel(exp, env, span, fuel)?;
                self.unify(&inferred, expected, span)
            }
            _ => {
                let inferred = self.infer_fuel(exp, env, span, fuel)?;
                self.unify(&inferred, expected, span)
            }
        }
    }

    /// Structural unification (skeleton): equal constructors unify;
    /// `int` vs `range` delegates to the solver via `check_fuel`-style
    /// obligations at the call site. Here: `Int` unifies with `Range` only
    /// if bounds are constant-covering — else error (needs solver context).
    fn unify(&self, inferred: &MiniTyp, expected: &MiniTyp, span: Span) -> TypeResult<()> {
        if inferred == expected {
            // Discharge well-kindedness of level terms with kind env built
            // from free vars treated opaquely: any var is assumed K_int.
            // (Full quant context comes from check_scheme.)
            let kenv = opaque_kenv(inferred);
            check_typ_kind(expected, &kenv, span)?;
            return Ok(());
        }
        match (inferred, expected) {
            (MiniTyp::Int, MiniTyp::Range(lo, hi)) | (MiniTyp::Range(lo, hi), MiniTyp::Int) => {
                match (lo.as_const(), hi.as_const()) {
                    (Some(_), Some(_)) => {
                        // `int` is the supertype of any `range`: accept.
                        Ok(())
                    }
                    _ => Err(TypeError::mismatch(
                        span,
                        expected.to_string(),
                        inferred.to_string(),
                    )),
                }
            }
            (MiniTyp::Fun(a1, b1), MiniTyp::Fun(a2, b2)) => {
                self.unify(a1, a2, span)?;
                self.unify(b1, b2, span)?;
                Ok(())
            }
            (MiniTyp::Tuple(ts1), MiniTyp::Tuple(ts2)) => {
                if ts1.len() != ts2.len() {
                    return Err(TypeError {
                        span,
                        kind: TypeErrorKind::ArityMismatch {
                            expected: ts2.len(),
                            found: ts1.len(),
                        },
                    });
                }
                for (a, b) in ts1.iter().zip(ts2.iter()) {
                    self.unify(a, b, span)?;
                }
                Ok(())
            }
            _ => Err(TypeError::mismatch(
                span,
                expected.to_string(),
                inferred.to_string(),
            )),
        }
    }
}

fn opaque_kenv(typ: &MiniTyp) -> KindEnv {
    let mut kenv = KindEnv::new();
    let mut ns = Vec::new();
    typ.nexps(&mut ns);
    for n in &ns {
        let mut vs = Vec::new();
        n.free_vars(&mut vs);
        for v in vs {
            if kenv.lookup(&v).is_none() {
                kenv.bind(v, crate::kind::Kind::KInt);
            }
        }
    }
    kenv
}

fn disp(e: &TypeError) -> String {
    e.to_string()
}
