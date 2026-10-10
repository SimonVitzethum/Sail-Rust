//! Optionale Z3-Anbindung (nur mit Feature `z3`, `z3`-Crate 0.21).
//!
//! `unsafe` steckt ausschliesslich in der `z3`-Dep; dieser Code bleibt
//! `#![forbid(unsafe_code)]`-konform (Kistenwurzel setzt es). Die 0.21-API
//! braucht keine expliziten `Context`-Handles mehr: AST-Knoten und
//! `Solver::new()` teilen sich den impliziten Thread-Kontext; Timeouts
//! laufen ueber [`Params`] (`"timeout"`, Millisekunden).
//! Jede `prove`-Instanz baut einen frischen Solver (Z3-Solver sind weder
//! `Send` noch `Sync`) — Wiederverwendung/Threadbindung leistet
//! [`crate::SolverPool`].
//!
//! ## NExp/NConstraint-Mapping (vollstaendig, siehe Operator-Tabelle in
//! `docs/nexp-solver-operators.md`)
//!
//! - Exakt: Konstanten (beliebig gross, Horner-Basis `2^32`), Variablen
//!   (gleicher Name = gleiche Z3-Konstante), `+`/`-`/`*`/unary `-`,
//!   Vergleiche, `=`/`!=`, `Set` (als `Or` von Gleichungen), `And`/`Or`/
//!   `Not`, `abs` und `If` (via `ite`).
//! - Konservativ (frische uninterpretierte Konstante, beweisgesund: eine
//!   freie Variable staerkt die zu beweisende Formel, verliert also nur
//!   Praezision Richtung `Unknown`, luegt nie `Valid`/`Invalid`):
//!   `div`/`mod` (Sail trunkiert wie Rust, Z3-`div`/`rem` weichen bei
//!   negativen Operanden ab), nicht-konstante `2^x` (Z3-`power` liefert
//!   `Real`, kein `Int`), unbekannte `Nexp_app`s, `Nexp_id`,
//!   `NC_app`-Praedikate, boolesche `NC_id`/`NC_var`.
//! - `2 ^ c` mit konstantem `c` in `[0, 127)` wird exakt gefaltet
//!   (gleiche Domaene wie [`sail_nexp::NExp::eval`]).

use z3::ast::{Ast, Bool, Int};
use z3::{Params, SatResult, Solver as Z3SolverInner};

use sail_nexp::{NConstraint, NExp};

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

    /// Beweist eine strukturierte `sail_nexp`-Formel direkt (ohne den
    /// Umweg ueber opake [`Constraint::Atom`]s).
    pub fn prove_nconstraint(&self, c: &NConstraint) -> SolverVerdict {
        check_valid(&nconstraint_to_bool(c), self.timeout_ms)
    }

    fn effective_timeout(&self, query: &SolverQuery) -> Option<u64> {
        query.timeout_ms.or(self.timeout_ms)
    }
}

/// Bildet einen [`NExp`] auf einen Z3-`Int`-AST ab.
///
/// Variablen mit gleichem Namen teilen sich die Z3-Konstante
/// (`Int::new_const` interniert per Namen); alles Nichtlineare ohne
/// exakte Entsprechung wird eine frische Konstante (siehe Modul-Doku).
pub fn nexp_to_int(e: &NExp) -> Int {
    match e {
        NExp::Constant(v) => int_literal(*v),
        NExp::Var(kid) | NExp::Id(kid) => Int::new_const(kid.as_str()),
        NExp::App(f, args) => nexp_app_to_int(f, args),
        NExp::If(c, t, b) => nconstraint_to_bool(c).ite(&nexp_to_int(t), &nexp_to_int(b)),
        NExp::Times(a, b) => Int::mul(&[nexp_to_int(a), nexp_to_int(b)]),
        NExp::Sum(a, b) => Int::add(&[nexp_to_int(a), nexp_to_int(b)]),
        NExp::Minus(a, b) => Int::sub(&[nexp_to_int(a), nexp_to_int(b)]),
        NExp::Exp(x) => nexp_exp_to_int(x),
        NExp::Neg(x) => nexp_to_int(x).unary_minus(),
    }
}

/// Bildet ein [`NConstraint`] auf einen Z3-`Bool`-AST ab.
pub fn nconstraint_to_bool(c: &NConstraint) -> Bool {
    match c {
        NConstraint::True => Bool::from_bool(true),
        NConstraint::False => Bool::from_bool(false),
        // Boolesche Atome als uninterpretierte Konstanten (beweisgesund:
        // freie Variablen koennen `Valid` nicht erschleichen).
        NConstraint::Id(name) | NConstraint::Var(name) => Bool::new_const(name.as_str()),
        NConstraint::Equal(a, b) => Ast::eq(&nexp_to_int(a), nexp_to_int(b)),
        NConstraint::NotEqual(a, b) => Ast::eq(&nexp_to_int(a), nexp_to_int(b)).not(),
        NConstraint::Ge(a, b) => nexp_to_int(a).ge(nexp_to_int(b)),
        NConstraint::Gt(a, b) => nexp_to_int(a).gt(nexp_to_int(b)),
        NConstraint::Le(a, b) => nexp_to_int(a).le(nexp_to_int(b)),
        NConstraint::Lt(a, b) => nexp_to_int(a).lt(nexp_to_int(b)),
        NConstraint::Set(n, vs) => {
            let lhs = nexp_to_int(n);
            let mut sorted = vs.clone();
            sorted.sort();
            sorted.dedup();
            let disjuncts: Vec<Bool> = sorted
                .iter()
                .map(|v| Ast::eq(&lhs, int_literal(*v)))
                .collect();
            if disjuncts.is_empty() {
                Bool::from_bool(false)
            } else {
                Bool::or(&disjuncts)
            }
        }
        // Uninterpretiertes Praedikat: frische Konstante (beweisgesund,
        // siehe Modul-Doku).
        NConstraint::App(_, _) => Bool::fresh_const("sail_nconstraint_app"),
        NConstraint::And(a, b) => Bool::and(&[nconstraint_to_bool(a), nconstraint_to_bool(b)]),
        NConstraint::Or(a, b) => Bool::or(&[nconstraint_to_bool(a), nconstraint_to_bool(b)]),
        NConstraint::Not(x) => nconstraint_to_bool(x).not(),
    }
}

/// Exaktes `i128`-Literal als Z3-`Int`.
///
/// `i64`/`u64`-Werte direkt; alles andere per Horner-Schema zur Basis
/// `2^32` (alle Teilwerte passen in `u64`), fuer Negative negiert. Z3-Ints
/// sind Bignums, daher ist das exakt — anders als `eval`, das bei
/// `>= 2^127` aufgibt.
fn int_literal(v: i128) -> Int {
    if let Ok(small) = i64::try_from(v) {
        return Int::from_i64(small);
    }
    if let Ok(big) = u64::try_from(v) {
        return Int::from_u64(big);
    }
    const BASE: u64 = 1 << 32;
    const MASK: u128 = (1 << 32) - 1;
    let neg = v < 0;
    let mag = v.unsigned_abs();
    let limb = |shift: u32| ((mag >> shift) & MASK) as u64;
    // `((l3*B + l2)*B + l1)*B + l0`, exakt per Konstruktion.
    let mut acc = Int::from_u64(limb(96));
    for shift in [64, 32, 0] {
        acc = Int::add(&[Int::mul(&[acc, Int::from_u64(BASE)]), Int::from_u64(limb(shift))]);
    }
    if neg {
        acc.unary_minus()
    } else {
        acc
    }
}

fn nexp_app_to_int(f: &str, args: &[NExp]) -> Int {
    match (f, args) {
        // `abs x = if x >= 0 then x else -x` (exakt via `ite`).
        ("abs", [x]) => {
            let x = nexp_to_int(x);
            x.ge(Int::from_i64(0)).ite(&x, &x.unary_minus())
        }
        // `div`/`mod` trunkieren in Sail wie Rust; Z3-`div`/`rem` weichen
        // bei negativen Operanden ab — daher frische Konstante
        // (beweisgesund, nur unpraeziser).
        ("div", _) => Int::fresh_const("sail_nexp_div"),
        ("mod", _) => Int::fresh_const("sail_nexp_mod"),
        _ => Int::fresh_const("sail_nexp_app"),
    }
}

fn nexp_exp_to_int(x: &NExp) -> Int {
    // `2 ^ c` exakt falten (gleiche Domaene wie `NExp::eval`); sonst
    // frische Konstante (`Int::power` lieferte `Real`, kein `Int`).
    if let NExp::Constant(e) = x {
        if (0..127).contains(e) {
            if let Some(v) = 1i128.checked_shl(*e as u32) {
                return int_literal(v);
            }
        }
    }
    Int::fresh_const("sail_nexp_exp")
}

impl Z3Solver {
    fn build_ast(c: &Constraint) -> Bool {
        match c {
            Constraint::True => Bool::from_bool(true),
            Constraint::False => Bool::from_bool(false),
            // Opakes Atom: uninterpretierte Konstante (Status quo aus der
            // stub-Aera; strukturierte Formeln bitte ueber
            // `prove_nconstraint` beweisen).
            Constraint::Atom(s) => Bool::new_const(s.as_str()),
            Constraint::Not(inner) => Self::build_ast(inner).not(),
            Constraint::And(cs) => {
                let parts: Vec<Bool> = cs.iter().map(Self::build_ast).collect();
                if parts.is_empty() {
                    Bool::from_bool(true)
                } else {
                    Bool::and(&parts)
                }
            }
            Constraint::Or(cs) => {
                let parts: Vec<Bool> = cs.iter().map(Self::build_ast).collect();
                if parts.is_empty() {
                    Bool::from_bool(false)
                } else {
                    Bool::or(&parts)
                }
            }
        }
    }
}

/// Gueltigkeitstest via Widerlegung: `Unsat(C)` heisst `C` ist widerlegt
/// (`Invalid`); `Unsat(¬C)` heisst `C` ist gueltig (`Valid`); sonst
/// `Unknown` (inkl. Timeout — terminiert immer).
///
/// Luegt nie: ein Timeout bei der ersten Pruefung faellt nur auf die
/// zweite zurueck — `Unsat(¬C)` allein impliziert bereits `Valid`.
fn check_valid(ast: &Bool, timeout: Option<u64>) -> SolverVerdict {
    if is_unsat(ast, timeout) {
        return SolverVerdict::Invalid;
    }
    // `is_unsat` lieh `ast` nur aus; Negation frisch aufbauen.
    if is_unsat(&ast.not(), timeout) {
        SolverVerdict::Valid
    } else {
        SolverVerdict::Unknown
    }
}

/// Eine `Unsat`-Anfrage mit frischem Solver und optionalem Timeout.
fn is_unsat(ast: &Bool, timeout: Option<u64>) -> bool {
    let solver = Z3SolverInner::new();
    if let Some(ms) = timeout {
        let mut params = Params::new();
        params.set_u32("timeout", ms.min(u32::MAX as u64) as u32);
        solver.set_params(&params);
    }
    solver.assert(ast);
    matches!(solver.check(), SatResult::Unsat)
}

impl Solver for Z3Solver {
    fn prove(&self, query: &SolverQuery) -> SolverVerdict {
        check_valid(&Self::build_ast(&query.constraint), self.effective_timeout(query))
    }

    fn name(&self) -> &'static str {
        "z3"
    }
}
