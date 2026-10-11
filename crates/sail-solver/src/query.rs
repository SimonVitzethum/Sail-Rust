//! Minimale Constraint-Darstellung fuer Solver-Anfragen.
//!
//! Lane 04/05 liefern echte NExp-Constraints; [`Constraint::from_nconstraint`]
//! bildet sie kanonisch (via `sail_nexp::normalize`) auf diese Struktur ab,
//! sodass gleiche Formeln gleiche Cache-Keys haben. Daneben traegt
//! [`Constraint::Atom`] weiterhin die normalisierte S-Expr als String. Der
//! Cache-Key ist die S-Expr plus Timeout, damit gleiche Anfragen gleiche
//! Keys haben.
//!
//! Lane 124: Kind-Variablen gehen mit ihrem *Namen* (aufgeloest durch den
//! uebergebenen [`Interner`]) in die Keys ein, nie mit der rohen
//! `Symbol`-Id (`kid.0`): Ids sind pro Interner-Tabelle vergeben, sodass
//! `kid.0` Kollisionen ueber Tabellen hinweg (gleiche Id, verschiedene
//! Variablen) und Divergenz bei gleicher Formel (gleiche Namen,
//! verschiedene Ids) erzeugt haette. Nicht aufloesbare Symbole fallen
//! auf `#<id>` zurueck (`#` kommt in Sail-Namen nicht vor, daher
//! kollisionsfrei).

use std::fmt;

use sail_ast::Interner;
use sail_nexp::{Kid, NConstraint as NC, NExp as NE, NTyp as NT, TypArg as NA};

/// Loest einen [`Kid`] zu seinem Cache-Key-Namen auf: der internerte
/// Name, oder `#<id>` wenn das Symbol in `it` unbekannt ist (kommt in
/// der Praxis nicht vor; `#` ist in Sail-Namen ungueltig, daher kann
/// der Fallback mit keinem echten Namen kollidieren).
fn kid_name(it: &impl Interner, kid: Kid) -> String {
    match it.resolve(kid) {
        Some(name) => name.to_owned(),
        None => format!("#{}", kid.0),
    }
}

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
    /// deterministische S-Expr-Atome. Kind-Variablen gehen mit ihrem
    /// Namen (aufgeloest durch `it`) ein: gleiche Formeln geben gleiche
    /// `Constraint`s (und damit gleiche Cache-Keys), unabhaengig von
    /// der `Symbol`-Vergabe der Tabelle.
    pub fn from_nconstraint(c: &NC, it: &impl Interner) -> Self {
        Self::from_normalized_nconstraint(&sail_nexp::normalize_constraint(c), it)
    }

    fn from_normalized_nconstraint(c: &NC, it: &impl Interner) -> Self {
        match c {
            NC::True => Constraint::True,
            NC::False => Constraint::False,
            NC::Id(name) => Constraint::atom(format!("(bid {name})")),
            NC::Var(kid) => Constraint::atom(format!("(bvar {})", kid_name(it, *kid))),
            NC::Equal(a, b) => {
                Constraint::atom(format!("(= {} {})", typarg_key(a, it), typarg_key(b, it)))
            }
            NC::NotEqual(a, b) => {
                Constraint::atom(format!("(!= {} {})", typarg_key(a, it), typarg_key(b, it)))
            }
            NC::Ge(a, b) => {
                Constraint::atom(format!("(>= {} {})", nexp_key(a, it), nexp_key(b, it)))
            }
            NC::Gt(a, b) => {
                Constraint::atom(format!("(> {} {})", nexp_key(a, it), nexp_key(b, it)))
            }
            NC::Le(a, b) => {
                Constraint::atom(format!("(<= {} {})", nexp_key(a, it), nexp_key(b, it)))
            }
            NC::Lt(a, b) => {
                Constraint::atom(format!("(< {} {})", nexp_key(a, it), nexp_key(b, it)))
            }
            NC::Set(n, vs) => {
                let mut vs = vs.clone();
                vs.sort();
                vs.dedup();
                let elems: Vec<String> = vs.iter().map(|v| v.to_string()).collect();
                Constraint::atom(format!("(in {} {})", nexp_key(n, it), elems.join(" ")))
            }
            NC::App(f, args) => {
                let argv: Vec<String> = args.iter().map(|a| typarg_key(a, it)).collect();
                Constraint::atom(format!("(pred {f} {})", argv.join(" ")))
            }
            NC::And(a, b) => Constraint::And(vec![
                Self::from_normalized_nconstraint(a, it),
                Self::from_normalized_nconstraint(b, it),
            ]),
            NC::Or(a, b) => Constraint::Or(vec![
                Self::from_normalized_nconstraint(a, it),
                Self::from_normalized_nconstraint(b, it),
            ]),
            NC::Not(x) => Constraint::Not(Box::new(Self::from_normalized_nconstraint(x, it))),
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

/// Kanonischer S-Expr-Schluessel eines [`TypArg`](sail_nexp::TypArg)
/// (Lane 108 `typ_arg`-Sorten): `NExp`-Args ueber [`nexp_key`] (der
/// vorab normalisiert), `Bool`-Args strukturell, `Typ`-Args ueber
/// [`ntyp_key`]. Variablen gehen namentlich (via `it`) ein, siehe
/// [`Constraint::from_nconstraint`].
fn typarg_key(a: &NA, it: &impl Interner) -> String {
    match a {
        NA::NExp(e) => nexp_key(e, it),
        NA::Bool(c) => Constraint::from_nconstraint(c, it).cache_key(),
        NA::Typ(t) => ntyp_key(t, it),
    }
}

/// Kanonischer S-Expr-Schluessel eines [`NTyp`](sail_nexp::NTyp):
/// strukturell, mit [`typarg_key`] fuer Argumente und
/// [`Constraint::from_nconstraint`] fuer `Exist`-Bedingungen (beide
/// idempotent normalisierend). Variablen gehen namentlich (via `it`)
/// ein, siehe [`Constraint::from_nconstraint`].
fn ntyp_key(t: &NT, it: &impl Interner) -> String {
    match t {
        NT::Unknown => "(typ-unknown)".to_owned(),
        NT::Id(name) => format!("(typ-id {name})"),
        NT::Var(kid) => format!("(typ-var {})", kid_name(it, *kid)),
        NT::Fn(args, ret) => {
            let argv: Vec<String> = args.iter().map(|a| ntyp_key(a, it)).collect();
            format!("(typ-fn ({}) {})", argv.join(" "), ntyp_key(ret, it))
        }
        NT::Bidir(a, b) => format!("(typ-bidir {} {})", ntyp_key(a, it), ntyp_key(b, it)),
        NT::Tuple(ts) => {
            let elems: Vec<String> = ts.iter().map(|t| ntyp_key(t, it)).collect();
            format!("(typ-tuple {})", elems.join(" "))
        }
        NT::App(f, args) => {
            let argv: Vec<String> = args.iter().map(|a| typarg_key(a, it)).collect();
            format!("(typ-app {f} {})", argv.join(" "))
        }
        NT::Exist(binders, c, ty) => {
            let bkeys: Vec<String> = binders
                .iter()
                .map(|b| format!("({:?} {})", b.kind, kid_name(it, b.kid)))
                .collect();
            format!(
                "(typ-exist ({}) {} {})",
                bkeys.join(" "),
                Constraint::from_nconstraint(c, it).cache_key(),
                ntyp_key(ty, it)
            )
        }
    }
}

/// Kanonischer S-Expr-Schluessel eines (bereits normalisierten)
/// [`NExp`](sail_nexp::NExp): `sail_nexp::normalize` vorab anwenden, damit
/// z. B. `b + a` und `a + b` denselben Schluessel erhalten. Variablen
/// gehen namentlich (via `it`) ein, siehe
/// [`Constraint::from_nconstraint`].
fn nexp_key(e: &NE, it: &impl Interner) -> String {
    let e = sail_nexp::normalize(e);
    match &e {
        NE::Id(name) => format!("(id {name})"),
        NE::Var(kid) => format!("(var {})", kid_name(it, *kid)),
        NE::Constant(v) => format!("(const {v})"),
        NE::App(f, args) => {
            let argv: Vec<String> = args.iter().map(|a| nexp_key(a, it)).collect();
            format!("(app {f} {})", argv.join(" "))
        }
        NE::If(c, t, b) => format!(
            "(if {} {} {})",
            Constraint::from_normalized_nconstraint(c, it).cache_key(),
            nexp_key(t, it),
            nexp_key(b, it)
        ),
        NE::Times(a, b) => format!("(* {} {})", nexp_key(a, it), nexp_key(b, it)),
        NE::Sum(a, b) => format!("(+ {} {})", nexp_key(a, it), nexp_key(b, it)),
        NE::Minus(a, b) => format!("(- {} {})", nexp_key(a, it), nexp_key(b, it)),
        NE::Exp(x) => format!("(pow2 {})", nexp_key(x, it)),
        NE::Neg(x) => format!("(neg {})", nexp_key(x, it)),
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
    fn not_is_structural_and_var_keys_use_names() {
        use sail_ast::{Interner as _, SimpleInterner};
        let mut ix = SimpleInterner::new();
        let n = ix.intern("n");
        let b = ix.intern("b");
        // `Not` bleibt strukturiert (kein opakes Atom).
        match Constraint::from_nconstraint(&NC::Not(Box::new(NC::True)), &ix) {
            Constraint::Not(x) => assert_eq!(*x, Constraint::True),
            other => panic!("expected Not, got {other:?}"),
        }
        // Keys nutzen Namen, nie die rohe Symbol-Id.
        assert_eq!(
            Constraint::from_nconstraint(&NC::Var(b), &ix),
            Constraint::atom("(bvar b)")
        );
        let key = Constraint::from_nconstraint(
            &NC::Ge(Box::new(NE::Var(n)), Box::new(NE::Constant(0))),
            &ix,
        )
        .cache_key();
        assert_eq!(key, "(atom (>= (var n) (const 0)))");
    }

    #[test]
    fn keys_are_stable_across_interners_and_separate_names() {
        use sail_ast::{Interner as _, SimpleInterner};
        // Lane 124: gleiche Namen in verschieden befuellten Tabellen
        // geben gleiche Keys (kein `kid.0`), verschiedene Namen geben
        // verschiedene Keys — auch bei gleicher Id-Position.
        let mut ix1 = SimpleInterner::new();
        let n1 = ix1.intern("n");
        let mut ix2 = SimpleInterner::new();
        let _other = ix2.intern("z");
        let n2 = ix2.intern("n");
        assert_ne!(n1, n2);
        let ge = |k: sail_nexp::Kid| NC::Ge(Box::new(NE::Var(k)), Box::new(NE::Constant(0)));
        assert_eq!(
            Constraint::from_nconstraint(&ge(n1), &ix1).cache_key(),
            Constraint::from_nconstraint(&ge(n2), &ix2).cache_key()
        );
        // Gleiche Id-Position (`0`), verschiedene Namen: keine Kollision.
        let mut ixa = SimpleInterner::new();
        let a = ixa.intern("a");
        let mut ixb = SimpleInterner::new();
        let b = ixb.intern("b");
        assert_eq!((a, b), (sail_ast::Symbol(0), sail_ast::Symbol(0)));
        assert_ne!(
            Constraint::from_nconstraint(&ge(a), &ixa).cache_key(),
            Constraint::from_nconstraint(&ge(b), &ixb).cache_key()
        );
    }
}
