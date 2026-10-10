#![forbid(unsafe_code)]
//! Solver-Kontext (Lane 107): echte prove-Pfade fuer `NC_set`/`NC_app`,
//! Kind-Umgebungen und `val`-/`register`-Schemes sowie `scattered`-Klauseln.
//!
//! Alles geht ausschliesslich durch den generischen
//! [`Solver`](crate::solver::Solver)-Trait (Regel `Z-TraitOnly`): Z3 hängt
//! in Lane 06/80 dahinter, `sail-ty` importiert hier — wie überall — kein
//! Backend. `Unknown` bleibt sound (Fehler an den Beweisstellen, nie ein
//! fabrizierter Beweis).
//!
//! Semantik: `assumptions ⊢ goal` wird als Implikation kodiert
//! (`¬(a₁ ∧ … ∧ aₙ) ∨ goal`, siehe [`implication`]) und als eine einzige
//! [`Solver::prove`]-Anfrage gestellt. Annahmen, die unter der Ziel-Art
//! (`KindEnv`) nicht wohlgekindet sind, fallen weg — das stärkt die zu
//! beweisende Formel (weniger annehmen = stärkere Behauptung) und kostet
//! nur Präzision Richtung `Unknown`, lügt nie `Proved`.

use crate::env::TypeEnv;
use crate::error::Span;
use crate::kind::{Kind, KindEnv, check_constraint_kind};
use crate::nexp::NConstraint;
use crate::solver::{ProveResult, Solver};
use crate::subsume::instantiate_scheme;

/// Baut eine [`KindEnv`] aus einem Quantor-Teleskop (`quants` eines
/// [`crate::nexp::TypQuant`]): die Art, unter der Scheme-Constraints und
/// -Rümpfe im Solver-Kontext geprüft werden.
#[must_use]
pub fn kenv_of_quants(quants: &[(String, Kind)]) -> KindEnv {
    let mut kenv = KindEnv::new();
    for (var, kind) in quants {
        kenv.bind(var.clone(), *kind);
    }
    kenv
}

/// Angenommene Vorbedingung eines `val`-Schemes: die frische (Q-Inst-)
/// Obligation aus [`instantiate_scheme`], `None` bei unbekanntem Namen.
#[must_use]
pub fn assumption_from_val(env: &TypeEnv, name: &str) -> Option<NConstraint> {
    env.lookup_val(name).map(|scheme| instantiate_scheme(scheme).0)
}

/// Angenommene Vorbedingung eines `register`-Schemes (wie
/// [`assumption_from_val`]).
#[must_use]
pub fn assumption_from_register(env: &TypeEnv, name: &str) -> Option<NConstraint> {
    env.lookup_register(name)
        .map(|scheme| instantiate_scheme(scheme).0)
}

/// `scattered function` im Solver-Kontext: die Klausel-Constraints werden
/// in Elaborationsreihenfolge konjugiert — Klausel `i` sieht die
/// Constraints von `clauses[..i]` als Annahmen (siehe
/// [`crate::check::BidirectionalChecker::check_scattered`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScatteredClauses {
    /// Funktionsname (`scattered function <name>`).
    pub name: String,
    /// Klausel-Constraints in Elaborationsreihenfolge.
    pub clauses: Vec<NConstraint>,
}

impl ScatteredClauses {
    /// Leere Klauselliste für `name`.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), clauses: Vec::new() }
    }

    /// Hängt ein Klausel-Constraint an (Elaborationsreihenfolge).
    pub fn push(&mut self, constraint: NConstraint) {
        self.clauses.push(constraint);
    }

    /// Konjunktion aller Klausel-Constraints (`None` bei leerer Liste).
    #[must_use]
    pub fn conjoin(&self) -> Option<NConstraint> {
        conjoin(&self.clauses)
    }
}

/// Konjunktion von `cs` in Reihenfolge (`None` bei leerer Liste).
fn conjoin(cs: &[NConstraint]) -> Option<NConstraint> {
    let mut acc: Option<NConstraint> = None;
    for c in cs {
        acc = Some(match acc {
            None => c.clone(),
            Some(prev) => NConstraint::And(Box::new(prev), Box::new(c.clone())),
        });
    }
    acc
}

/// Kodiert `assumptions ⊢ goal` als eine Formel: `¬(a₁ ∧ … ∧ aₙ) ∨ goal`.
/// Ohne Annahmen ist das Ergebnis `goal` selbst.
#[must_use]
pub fn implication(assumptions: &[NConstraint], goal: &NConstraint) -> NConstraint {
    match conjoin(assumptions) {
        None => goal.clone(),
        Some(all) => NConstraint::Or(
            Box::new(NConstraint::Not(Box::new(all))),
            Box::new(goal.clone()),
        ),
    }
}

/// Angesammelte Solver-Annahmen aus `val`-/`register`-Schemes und
/// `scattered`-Klauseln (siehe [`assumption_from_val`],
/// [`assumption_from_register`], [`ScatteredClauses`]).
#[derive(Clone, Debug, Default)]
pub struct SolverContext {
    assumptions: Vec<NConstraint>,
}

impl SolverContext {
    /// Leerer Kontext (`prove_in_context` ≡ `prove` plus Artprüfung).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Nimmt eine Annahme auf (Reihenfolge bleibt für [`implication`]).
    pub fn assume(&mut self, constraint: NConstraint) {
        self.assumptions.push(constraint);
    }

    /// Nimmt die [`assumption_from_val`]-Obligation für `name` auf;
    /// `false` bei unbekanntem Namen (keine Annahme aufgenommen).
    pub fn assume_val(&mut self, env: &TypeEnv, name: &str) -> bool {
        match assumption_from_val(env, name) {
            Some(c) => {
                self.assume(c);
                true
            }
            None => false,
        }
    }

    /// Nimmt die [`assumption_from_register`]-Obligation für `name` auf;
    /// `false` bei unbekanntem Namen.
    pub fn assume_register(&mut self, env: &TypeEnv, name: &str) -> bool {
        match assumption_from_register(env, name) {
            Some(c) => {
                self.assume(c);
                true
            }
            None => false,
        }
    }

    /// Nimmt alle Klausel-Constraints von `scattered` auf
    /// (Elaborationsreihenfolge).
    pub fn assume_scattered(&mut self, scattered: &ScatteredClauses) {
        self.assumptions.extend(scattered.clauses.iter().cloned());
    }

    /// Bisherige Annahmen in Aufnahmereihenfolge.
    #[must_use]
    pub fn assumptions(&self) -> &[NConstraint] {
        &self.assumptions
    }

    /// Anzahl der Annahmen.
    #[must_use]
    pub fn len(&self) -> usize {
        self.assumptions.len()
    }

    /// `true` bei leerem Kontext.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.assumptions.is_empty()
    }
}

/// Beweist `goal` unter `ctx` mit `solver` — ausschließlich via
/// [`Solver::prove`] (ein Aufruf auf [`implication`]).
///
/// Pfade (alle total, alle durch den Trait):
/// - `goal` muss unter `kenv` die Art `K_bool` haben, sonst `Unknown`
///   (Kind-Env-Pfad statt Stub-Blindheit).
/// - Annahmen, die unter `kenv` nicht wohlgekindet sind, fallen weg
///   (sound: weniger annehmen stärkt die Behauptung, kostet nur
///   Präzision).
/// - Leerer Kontext stellt exakt `solver.prove(goal)`.
#[must_use]
pub fn prove_in_context<S: Solver>(
    solver: &S,
    kenv: &KindEnv,
    ctx: &SolverContext,
    goal: &NConstraint,
    span: Span,
) -> ProveResult {
    if check_constraint_kind(goal, kenv, span).is_err() {
        return ProveResult::Unknown;
    }
    let well_kinded: Vec<NConstraint> = ctx
        .assumptions
        .iter()
        .filter(|a| check_constraint_kind(a, kenv, span).is_ok())
        .cloned()
        .collect();
    solver.prove(&implication(&well_kinded, goal))
}
