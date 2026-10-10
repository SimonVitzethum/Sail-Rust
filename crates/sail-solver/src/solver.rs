//! Lane-05-kompatibler `Solver`-Trait: `prove(nc) -> Valid/Invalid/Unknown`.

use crate::query::SolverQuery;

/// Ergebnis eines Beweisversuchs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SolverVerdict {
    /// Formel gueltig (bewiesen).
    Valid,
    /// Formel widerlegt (Negation bewiesen / Formel unsat).
    Invalid,
    /// Unbekannt (Timeout, Stub, ausserhalb der Theorie).
    Unknown,
}

impl SolverVerdict {
    /// Lane-05-Sicht: `true` genau bei bewiesener Gueltigkeit.
    pub fn is_valid(self) -> bool {
        matches!(self, SolverVerdict::Valid)
    }
}

/// Solver-Schnittstelle des Typcheckers (Lane 05).
///
/// Implementierungen muessen terminieren (Timeout/Unknown statt Haengen)
/// und `Send` sein, damit ein Solver exklusiv an einen Thread
/// ausgeliehen werden kann ([`crate::SolverPool`]). Gleichzeitiges Teilen
/// einer Instanz ist nicht vorgesehen: 1 Solver pro Thread.
pub trait Solver: Send {
    /// Versucht, `query` zu beweisen. Terminiert immer.
    fn prove(&self, query: &SolverQuery) -> SolverVerdict;

    /// Backend-Name fuer Diagnose (z. B. `"stub"`, `"z3"`).
    fn name(&self) -> &'static str;
}
