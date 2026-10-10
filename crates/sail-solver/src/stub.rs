//! Reiner Stub-Solver (Default, keine C-Dep).
//!
//! Terminiert immer: Konstanten werden gefaltet (`true` -> `Valid`,
//! `false` -> `Invalid`), alles andere ist [`SolverVerdict::Unknown`].
//! Dient Lane 05 als Default-Fallback und als Referenz in Diff-Tests.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::query::{Constraint, SolverQuery};
use crate::solver::{Solver, SolverVerdict};

/// Stub-Solver ohne Abhaengigkeiten.
#[derive(Debug, Default)]
pub struct StubSolver {
    queries: AtomicU64,
}

impl StubSolver {
    pub fn new() -> Self {
        StubSolver {
            queries: AtomicU64::new(0),
        }
    }

    /// Anzahl bisheriger `prove`-Aufrufe.
    pub fn query_count(&self) -> u64 {
        self.queries.load(Ordering::Relaxed)
    }

    fn fold(c: &Constraint) -> Option<bool> {
        match c {
            Constraint::True => Some(true),
            Constraint::False => Some(false),
            Constraint::Atom(_) => None,
            Constraint::Not(inner) => Self::fold(inner).map(|b| !b),
            Constraint::And(cs) => {
                let mut any_unknown = false;
                for c in cs {
                    match Self::fold(c) {
                        Some(false) => return Some(false),
                        None => any_unknown = true,
                        Some(true) => {}
                    }
                }
                if any_unknown {
                    None
                } else {
                    Some(true)
                }
            }
            Constraint::Or(cs) => {
                let mut any_unknown = false;
                for c in cs {
                    match Self::fold(c) {
                        Some(true) => return Some(true),
                        None => any_unknown = true,
                        Some(false) => {}
                    }
                }
                if any_unknown {
                    None
                } else {
                    Some(false)
                }
            }
        }
    }
}

impl Solver for StubSolver {
    fn prove(&self, query: &SolverQuery) -> SolverVerdict {
        self.queries.fetch_add(1, Ordering::Relaxed);
        match Self::fold(&query.constraint) {
            Some(true) => SolverVerdict::Valid,
            Some(false) => SolverVerdict::Invalid,
            None => SolverVerdict::Unknown,
        }
    }

    fn name(&self) -> &'static str {
        "stub"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_terminates_with_unknown_on_opaque() {
        let s = StubSolver::new();
        let v = s.prove(&SolverQuery::new(Constraint::atom("x > 0")));
        assert_eq!(v, SolverVerdict::Unknown);
        assert!(!v.is_valid());
        assert_eq!(s.query_count(), 1);
    }

    #[test]
    fn stub_folds_constants() {
        let s = StubSolver::new();
        assert_eq!(
            s.prove(&SolverQuery::new(Constraint::True)),
            SolverVerdict::Valid
        );
        assert_eq!(
            s.prove(&SolverQuery::new(Constraint::False)),
            SolverVerdict::Invalid
        );
        assert_eq!(
            s.prove(&SolverQuery::new(Constraint::Not(Box::new(
                Constraint::True
            )))),
            SolverVerdict::Invalid
        );
    }
}
