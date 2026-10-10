//! Optionale Z3-Anbindung (nur mit Feature `z3`).
//!
//! `unsafe` steckt ausschliesslich in der `z3`-Dep; dieser Code bleibt
//! `#![forbid(unsafe_code)]`-konform. Jede Instanz baut pro `prove` frische
//! `Config`/`Context`/`Solver` (Z3-Kontexte sind nicht `Sync`) —
//! Wiederverwendung/Threadbindung leistet [`crate::SolverPool`].

use z3::ast::Bool;
use z3::{Config, Context, SatResult, Solver as Z3SolverInner};

use crate::query::{Constraint, SolverQuery};
use crate::solver::{Solver, SolverVerdict};

/// Z3-gestuetzter Solver.
#[derive(Debug, Default)]
pub struct Z3Solver {
    timeout_ms: Option<u64>,
}

impl Z3Solver {
    pub fn new() -> Self {
        Z3Solver { timeout_ms: None }
    }

    pub fn with_timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = Some(ms);
        self
    }

    fn build_ast<'ctx>(ctx: &'ctx Context, c: &Constraint) -> Bool<'ctx> {
        match c {
            Constraint::True => Bool::from_bool(ctx, true),
            Constraint::False => Bool::from_bool(ctx, false),
            Constraint::Atom(s) => Bool::new_const(ctx, s.clone()),
            Constraint::Not(inner) => Self::build_ast(ctx, inner).not(),
            Constraint::And(cs) => {
                let parts: Vec<Bool<'ctx>> =
                    cs.iter().map(|x| Self::build_ast(ctx, x)).collect();
                let refs: Vec<&Bool<'ctx>> = parts.iter().collect();
                Bool::and(ctx, &refs)
            }
            Constraint::Or(cs) => {
                let parts: Vec<Bool<'ctx>> =
                    cs.iter().map(|x| Self::build_ast(ctx, x)).collect();
                let refs: Vec<&Bool<'ctx>> = parts.iter().collect();
                Bool::or(ctx, &refs)
            }
        }
    }

    fn check_sat(ctx: &Context, c: &Constraint) -> SatResult {
        let solver = Z3SolverInner::new(ctx);
        solver.assert(&Self::build_ast(ctx, c));
        solver.check()
    }
}

impl Solver for Z3Solver {
    fn prove(&self, query: &SolverQuery) -> SolverVerdict {
        let mut cfg = Config::new();
        let timeout = query.timeout_ms.or(self.timeout_ms);
        if let Some(ms) = timeout {
            cfg.set_timeout_msec(ms);
        }
        let ctx = Context::new(&cfg);
        // Widerlegungstest: C unsat => Invalid.
        match Self::check_sat(&ctx, &query.constraint) {
            SatResult::Unsat => SolverVerdict::Invalid,
            SatResult::Unknown => SolverVerdict::Unknown,
            SatResult::Sat => {
                // Gueltigkeitstest: ¬C unsat => Valid, sonst Unknown.
                let neg = Constraint::Not(Box::new(query.constraint.clone()));
                match Self::check_sat(&ctx, &neg) {
                    SatResult::Unsat => SolverVerdict::Valid,
                    _ => SolverVerdict::Unknown,
                }
            }
        }
    }

    fn name(&self) -> &'static str {
        "z3"
    }
}
