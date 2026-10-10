//! Minimale Constraint-Darstellung fuer Solver-Anfragen.
//!
//! Lane 04/05 liefern spaeter echte NExp-Constraints; bis dahin traegt
//! [`Constraint::Atom`] die normalisierte S-Expr als String. Der Cache-Key
//! ist die S-Expr plus Timeout, damit gleiche Anfragen gleiche Keys haben.

use std::fmt;

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
}
