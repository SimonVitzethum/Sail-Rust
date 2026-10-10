//! Minimale Constraint-Darstellung fuer Solver-Anfragen.
//!
//! Lane 04/05 liefern echte NExp-Constraints; [`Constraint::from_nconstraint`]
//! bildet sie kanonisch (via `sail_nexp::normalize`) auf diese Struktur ab,
//! sodass gleiche Formeln gleiche Cache-Keys haben. Daneben traegt
//! [`Constraint::Atom`] weiterhin die normalisierte S-Expr als String. Der
//! Cache-Key ist die S-Expr plus Timeout, damit gleiche Anfragen gleiche
//! Keys haben.

use std::fmt;

use sail_nexp::{NConstraint as NC, NExp as NE};

/// Einzelne Constraint-Formel (strukturiert, hashbar).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Constraint {
    /// Tautologie.
    True,
    /// Kontradiktion.
    False,
    /// Opake normalisierte Formel (S-Expr aus Lane 04/05).
    Atom(String),
    /// Negation.
    Not(Box<Constraint>),
    /// Konjunktion.
    And(Vec<Constraint>),
    /// Disjunktion.
    Or(Vec<Constraint>),
}

impl Constraint {
    /// Opakes Atom aus normalisierter S-Expr.
    pub fn atom(s: impl Into<String>) -> Self {
        Constraint::Atom(s.into())
    }

    /// Strukturierte Bruecke aus `sail_nexp`: normalisiert erst mit
    /// `sail_nexp::normalize_constraint` (kanonische Operandenordnung),
    /// bildet `And`/`Or`/`Not` strukturell ab und alle Blaetter als
    /// deterministische S-Expr-Atome. Gleiche NConstraint-Formeln geben
    /// gleiche `Constraint`s (und damit gleiche Cache-Keys).
    pub fn from_nconstraint(c: &NC) -> Self {
        Self::from_normalized_nconstraint(&sail_nexp::normalize_constraint(c))
    }

    fn from_normalized_nconstraint(c: &NC) -> Self {
        match c {
            NC::True => Constraint::True,
            NC::False => Constraint::False,
            NC::Id(name) => Constraint::atom(format!("(bid {name})")),
            NC::Var(kid) => Constraint::atom(format!("(bvar {})", kid.0)),
            NC::Equal(a, b) => {
                Constraint::atom(format!("(= {} {})", nexp_key(a), nexp_key(b)))
            }
            NC::NotEqual(a, b) => {
                Constraint::atom(format!("(!= {} {})", nexp_key(a), nexp_key(b)))
            }
            NC::Ge(a, b) => {
                Constraint::atom(format!("(>= {} {})", nexp_key(a), nexp_key(b)))
            }
            NC::Gt(a, b) => {
                Constraint::atom(format!("(> {} {})", nexp_key(a), nexp_key(b)))
            }
            NC::Le(a, b) => {
                Constraint::atom(format!("(<= {} {})", nexp_key(a), nexp_key(b)))
            }
            NC::Lt(a, b) => {
                Constraint::atom(format!("(< {} {})", nexp_key(a), nexp_key(b)))
            }
            NC::Set(n, vs) => {
                let mut vs = vs.clone();
                vs.sort();
                vs.dedup();
                let elems: Vec<String> = vs.iter().map(|v| v.to_string()).collect();
                Constraint::atom(format!("(in {} {})", nexp_key(n), elems.join(" ")))
            }
            NC::App(f, args) => {
                let argv: Vec<String> = args.iter().map(nexp_key).collect();
                Constraint::atom(format!("(pred {f} {})", argv.join(" ")))
            }
            NC::And(a, b) => Constraint::And(vec![
                Self::from_normalized_nconstraint(a),
                Self::from_normalized_nconstraint(b),
            ]),
            NC::Or(a, b) => Constraint::Or(vec![
                Self::from_normalized_nconstraint(a),
                Self::from_normalized_nconstraint(b),
            ]),
            NC::Not(x) => Constraint::Not(Box::new(Self::from_normalized_nconstraint(x))),
        }
    }

    /// S-Expr-Normalform als Cache-Key-Baustein.
    pub fn cache_key(&self) -> String {
        match self {
            Constraint::True => "true".to_owned(),
            Constraint::False => "false".to_owned(),
            Constraint::Atom(s) => format!("(atom {s})"),
            Constraint::Not(c) => format!("(not {})", c.cache_key()),
            Constraint::And(cs) => {
                let mut k = String::from("(and");
                for c in cs {
                    k.push(' ');
                    k.push_str(&c.cache_key());
                }
                k.push(')');
                k
            }
            Constraint::Or(cs) => {
                let mut k = String::from("(or");
                for c in cs {
                    k.push(' ');
                    k.push_str(&c.cache_key());
                }
                k.push(')');
                k
            }
        }
    }
}

impl fmt::Display for Constraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.cache_key())
    }
}

/// Kanonischer S-Expr-Schluessel eines (bereits normalisierten)
/// [`NExp`](sail_nexp::NExp): `sail_nexp::normalize` vorab anwenden, damit
/// z. B. `b + a` und `a + b` denselben Schluessel erhalten.
fn nexp_key(e: &NE) -> String {
    let e = sail_nexp::normalize(e);
    match &e {
        NE::Id(name) => format!("(id {name})"),
        NE::Var(kid) => format!("(var {})", kid.0),
        NE::Constant(v) => format!("(const {v})"),
        NE::App(f, args) => {
            let argv: Vec<String> = args.iter().map(nexp_key).collect();
            format!("(app {f} {})", argv.join(" "))
        }
        NE::If(c, t, b) => format!(
            "(if {} {} {})",
            Constraint::from_normalized_nconstraint(c).cache_key(),
            nexp_key(t),
            nexp_key(b)
        ),
        NE::Times(a, b) => format!("(* {} {})", nexp_key(a), nexp_key(b)),
        NE::Sum(a, b) => format!("(+ {} {})", nexp_key(a), nexp_key(b)),
        NE::Minus(a, b) => format!("(- {} {})", nexp_key(a), nexp_key(b)),
        NE::Exp(x) => format!("(pow2 {})", nexp_key(x)),
        NE::Neg(x) => format!("(neg {})", nexp_key(x)),
    }
}

/// Eine Solver-Anfrage: Formel plus optionalem Timeout.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SolverQuery {
    /// Zu beweisende Formel.
    pub constraint: Constraint,
    /// Timeout in Millisekunden (geht in den Cache-Key ein).
    pub timeout_ms: Option<u64>,
}

impl SolverQuery {
    pub fn new(constraint: Constraint) -> Self {
        SolverQuery {
            constraint,
            timeout_ms: None,
        }
    }

    pub fn with_timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = Some(ms);
        self
    }

    /// Eindeutiger Cache-Key (Formel + Timeout).
    pub fn cache_key(&self) -> String {
        match self.timeout_ms {
            Some(ms) => format!("{} @{}ms", self.constraint.cache_key(), ms),
            None => self.constraint.cache_key(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_is_stable_and_timeout_sensitive() {
        let a = SolverQuery::new(Constraint::atom("x > 0"));
        let b = SolverQuery::new(Constraint::atom("x > 0"));
        assert_eq!(a.cache_key(), b.cache_key());
        let c = SolverQuery::new(Constraint::atom("x > 0")).with_timeout(100);
        assert_ne!(a.cache_key(), c.cache_key());
    }

    #[test]
    fn not_is_structural_and_var_keys_use_symbol_id() {
        // `Not` bleibt strukturiert (kein opakes Atom).
        match Constraint::from_nconstraint(&NC::Not(Box::new(NC::True))) {
            Constraint::Not(x) => assert_eq!(*x, Constraint::True),
            other => panic!("expected Not, got {other:?}"),
        }
        // Internierte Kids haben kein `Display`: Keys nutzen die rohe
        // Symbol-Id (deterministisch je Interner).
        assert_eq!(
            Constraint::from_nconstraint(&NC::Var(sail_nexp::Kid(3))),
            Constraint::atom("(bvar 3)")
        );
        let key = Constraint::from_nconstraint(&NC::Ge(
            Box::new(NE::Var(sail_nexp::Kid(1))),
            Box::new(NE::Constant(0)),
        ))
        .cache_key();
        assert_eq!(key, "(atom (>= (var 1) (const 0)))");
    }
}
