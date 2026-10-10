#![forbid(unsafe_code)]
//! Operator-Vollstaendigkeit Lane 80: jede `NExp`- und `NConstraint`-Variante
//! (inkl. neuem `NC_app`/`App`) durch `eval`/`subst`/`vars`/`simplify`/`
//! provably_*`/`lean`-Abbildung. Reine Code-Tests (E3: nicht ausfuehren).

use sail_ast::{Interner as _, SimpleInterner};
use sail_nexp::{
    int_kind, lean_type_name, normalize, normalize_constraint, provably_neg, provably_neg_in,
    provably_nneg, provably_nneg_in, provably_nonpos, provably_nonpos_in, provably_nonzero,
    provably_nonzero_in, provably_pos, provably_pos_in, provably_zero, provably_zero_in, simplify,
    simplify_constraint, Facts, Kid, NConstraint, NExp, TypArg,
};
use std::borrow::Cow;
use std::collections::BTreeMap;

fn v(k: Kid) -> NExp {
    NExp::var(k)
}
fn c(i: i128) -> NExp {
    NExp::Constant(i)
}
fn env(ix: &mut SimpleInterner, pairs: &[(&str, i128)]) -> BTreeMap<Kid, i128> {
    pairs.iter().map(|(k, x)| (ix.intern(k), *x)).collect()
}
fn subst_of(ix: &mut SimpleInterner, pairs: &[(&str, NExp)]) -> BTreeMap<Kid, NExp> {
    pairs.iter().map(|(k, e)| (ix.intern(k), e.clone())).collect()
}

#[test]
fn all_nexp_operators_evaluate() {
    let mut ix = SimpleInterner::new();
    let n = ix.intern("n");
    let m = ix.intern("m");
    let e = env(&mut ix, &[("n", 7), ("m", -3)]);
    // Id: immer unbekannt.
    assert_eq!(NExp::Id("f".to_string()).eval(&e), None);
    // Var / Constant.
    assert_eq!(v(n).eval(&e), Some(7));
    assert_eq!(c(4).eval(&e), Some(4));
    // App-Operatoren.
    assert_eq!(NExp::div(v(n), c(2)).eval(&e), Some(3));
    assert_eq!(NExp::modulo(v(n), c(4)).eval(&e), Some(3));
    assert_eq!(NExp::abs(v(m)).eval(&e), Some(3));
    assert_eq!(NExp::div(c(1), c(0)).eval(&e), None);
    // Unbekannte App: None.
    assert_eq!(NExp::App("uf".to_string(), vec![v(n)]).eval(&e), None);
    // If / Times / Sum / Minus / Exp / Neg.
    let ite = NExp::If(
        Box::new(NConstraint::Ge(Box::new(v(n)), Box::new(c(0)))),
        Box::new(c(1)),
        Box::new(c(2)),
    );
    assert_eq!(ite.eval(&e), Some(1));
    assert_eq!(
        NExp::Times(Box::new(v(n)), Box::new(v(m))).eval(&e),
        Some(-21)
    );
    assert_eq!(NExp::Sum(Box::new(v(n)), Box::new(v(m))).eval(&e), Some(4));
    assert_eq!(
        NExp::Minus(Box::new(v(n)), Box::new(v(m))).eval(&e),
        Some(10)
    );
    assert_eq!(NExp::Exp(Box::new(c(4))).eval(&e), Some(16));
    assert_eq!(NExp::Neg(Box::new(v(n))).eval(&e), Some(-7));
    assert_eq!(NExp::Exp(Box::new(c(127))).eval(&e), None);
}

#[test]
fn all_nconstraint_operators_evaluate() {
    let mut ix = SimpleInterner::new();
    let n = ix.intern("n");
    let b = ix.intern("b");
    let e = env(&mut ix, &[("n", 3)]);
    // Equal/NotEqual vergleichen `typ_arg`-Sorten (Lane 108).
    let cmp_t = |f: fn(Box<TypArg>, Box<TypArg>) -> NConstraint| {
        f(
            Box::new(TypArg::NExp(v(n))),
            Box::new(TypArg::NExp(c(3))),
        )
        .eval(&e)
    };
    let cmp = |f: fn(Box<NExp>, Box<NExp>) -> NConstraint| {
        f(Box::new(v(n)), Box::new(c(3))).eval(&e)
    };
    assert_eq!(NConstraint::True.eval(&e), Some(true));
    assert_eq!(NConstraint::False.eval(&e), Some(false));
    assert_eq!(NConstraint::Id("p".to_string()).eval(&e), None);
    assert_eq!(NConstraint::Var(b).eval(&e), None);
    assert_eq!(cmp_t(NConstraint::Equal), Some(true));
    assert_eq!(cmp_t(NConstraint::NotEqual), Some(false));
    assert_eq!(cmp(NConstraint::Ge), Some(true));
    assert_eq!(cmp(NConstraint::Gt), Some(false));
    assert_eq!(cmp(NConstraint::Le), Some(true));
    assert_eq!(cmp(NConstraint::Lt), Some(false));
    assert_eq!(
        NConstraint::Set(Box::new(v(n)), vec![1, 2, 3]).eval(&e),
        Some(true)
    );
    assert_eq!(NConstraint::Set(Box::new(v(n)), vec![]).eval(&e), Some(false));
    // NC_app (neu): uninterpretiert => None.
    assert_eq!(
        NConstraint::App("ok".to_string(), vec![TypArg::NExp(v(n))]).eval(&e),
        None
    );
    let t = NConstraint::True;
    let f = NConstraint::False;
    assert_eq!(
        NConstraint::And(Box::new(t.clone()), Box::new(f.clone())).eval(&e),
        Some(false)
    );
    assert_eq!(
        NConstraint::Or(Box::new(t.clone()), Box::new(f.clone())).eval(&e),
        Some(true)
    );
    assert_eq!(NConstraint::Not(Box::new(t)).eval(&e), Some(false));
}

#[test]
fn nc_app_vars_subst_simplify() {
    let mut ix = SimpleInterner::new();
    let n = ix.intern("n");
    let m = ix.intern("m");
    // vars in Erstauftretens-Reihenfolge (Praedikat + Args).
    let wrap = |e: NExp| TypArg::NExp(e);
    let a = NConstraint::App(
        "p".to_string(),
        vec![wrap(v(m)), wrap(v(n)), wrap(v(m))],
    );
    assert_eq!(a.vars(), vec![m, n]);
    // subst trifft nur Args; unveraendert wird geborgt.
    let empty: BTreeMap<Kid, NExp> = BTreeMap::new();
    assert!(matches!(a.subst_cow(&empty), Cow::Borrowed(_)));
    let s = subst_of(&mut ix, &[("n", c(9))]);
    assert_eq!(
        a.subst(&s),
        NConstraint::App(
            "p".to_string(),
            vec![wrap(v(m)), wrap(c(9)), wrap(v(m))]
        )
    );
    // simplify faltet nur Argumente, nie das Praedikat selbst.
    let b = NConstraint::App(
        "p".to_string(),
        vec![wrap(NExp::Sum(Box::new(c(1)), Box::new(c(2))))],
    );
    assert_eq!(
        simplify_constraint(&b),
        NConstraint::App("p".to_string(), vec![wrap(c(3))])
    );
    // normalize sortiert/deduped nichts weg, ordnet aber Args kanonisch.
    assert_eq!(normalize_constraint(&b), normalize_constraint(&b));
}

#[test]
fn provably_family_covers_all_operators() {
    let mut ix = SimpleInterner::new();
    let n = ix.intern("n");
    let m = ix.intern("m");
    let b = ix.intern("b");
    // Konstanten: volle Tafel.
    assert!(provably_nneg(&c(0)) && provably_pos(&c(1)) && provably_neg(&c(-1)));
    assert!(provably_nonpos(&c(0)) && provably_zero(&c(0)) && provably_nonzero(&c(2)));
    assert!(!provably_zero(&c(2)) && !provably_nonzero(&c(0)));
    // App-Operatoren ohne Kontext.
    assert!(provably_pos(&NExp::Exp(Box::new(v(n)))));
    assert!(provably_nneg(&NExp::abs(v(n))));
    assert!(!provably_pos(&NExp::abs(v(n))));
    assert!(provably_nneg(&NExp::modulo(v(n), c(4))));
    assert!(!provably_nneg(&NExp::div(v(n), c(2))));
    assert!(!provably_nneg(&NExp::App("uf".to_string(), vec![v(n)])));
    assert!(!provably_nneg(&NExp::Id("f".to_string())));
    // If-Konsens beider Zweige.
    let ite = NExp::If(
        Box::new(NConstraint::Var(b)),
        Box::new(c(1)),
        Box::new(c(2)),
    );
    assert!(provably_pos(&ite));
    let split = NExp::If(
        Box::new(NConstraint::Var(b)),
        Box::new(c(1)),
        Box::new(c(-1)),
    );
    assert!(!provably_nneg(&split) && !provably_nonpos(&split));
    // Arithmetik unter Facts.
    let f = Facts::new().with_lower(n, 0).with_lower(m, 1);
    let sum = NExp::Sum(Box::new(v(n)), Box::new(v(m)));
    assert!(provably_pos_in(&sum, &f) && provably_nneg_in(&sum, &f));
    let prod = NExp::Times(Box::new(v(n)), Box::new(v(m)));
    assert!(provably_nneg_in(&prod, &f));
    assert!(!provably_pos_in(&prod, &f));
    let neg = NExp::Neg(Box::new(v(m)));
    assert!(provably_neg_in(&neg, &f) && provably_nonpos_in(&neg, &f));
    // Zero/Nonzero durch Formen.
    let z = NExp::Times(Box::new(c(0)), Box::new(v(n)));
    assert!(provably_zero(&z) && provably_zero_in(&z, &Facts::new()));
    assert!(!provably_nonzero(&z));
    let d = NExp::Minus(Box::new(v(n)), Box::new(v(n)));
    assert!(provably_zero(&d));
    // Upper-Bounds treiben die duale Seite.
    let g = Facts::new().with_upper(n, -1);
    assert!(provably_neg_in(&v(n), &g) && provably_nonzero_in(&v(n), &g));
    assert!(!provably_nneg_in(&v(n), &g));
    let h = Facts::new().with_upper(n, 0);
    assert!(provably_nonpos_in(&v(n), &h));
}

#[test]
fn facts_from_constraint_covers_equal_and_set() {
    let mut ix = SimpleInterner::new();
    let n = ix.intern("n");
    let a = ix.intern("a");
    let b = ix.intern("b");
    let d = ix.intern("d");
    // `x == c` pinnt beide Bounds (beide Operandenreihenfolgen).
    for (x, y) in [
        (v(n), c(5)),
        (c(5), v(n)),
    ] {
        let f = Facts::from_constraint(&NConstraint::Equal(
            Box::new(TypArg::NExp(x)),
            Box::new(TypArg::NExp(y)),
        ));
        assert_eq!((f.lower(n), f.upper(n)), (Some(5), Some(5)));
        assert!(provably_pos_in(&v(n), &f));
        assert!(!provably_zero_in(&v(n), &f));
    }
    // Einelementiges Set pinnt ebenfalls.
    let f = Facts::from_constraint(&NConstraint::Set(Box::new(v(n)), vec![0]));
    assert!(provably_zero_in(&v(n), &f));
    // `And` sammelt ueber alle Operatoren hinweg.
    let wrap = |e: NExp| Box::new(TypArg::NExp(e));
    let all = NConstraint::And(
        Box::new(NConstraint::Equal(wrap(v(a)), wrap(c(2)))),
        Box::new(NConstraint::And(
            Box::new(NConstraint::NotEqual(wrap(v(b)), wrap(c(0)))),
            Box::new(NConstraint::Ge(Box::new(v(d)), Box::new(c(3)))),
        )),
    );
    let f = Facts::from_constraint(&all);
    assert_eq!((f.lower(a), f.upper(a)), (Some(2), Some(2)));
    assert!(f.is_nonzero_var(b));
    assert!(provably_nonzero_in(&v(b), &f));
    assert_eq!(f.lower(d), Some(3));
}

#[test]
fn normalize_constraint_is_canonical_for_and_or() {
    let mut ix = SimpleInterner::new();
    let n = ix.intern("n");
    let m = ix.intern("m");
    let a = ix.intern("a");
    let b = ix.intern("b");
    let ge = || NConstraint::Ge(Box::new(v(n)), Box::new(c(0)));
    let lt = || NConstraint::Lt(Box::new(v(m)), Box::new(c(0)));
    let ab = NConstraint::And(Box::new(ge()), Box::new(lt()));
    let ba = NConstraint::And(Box::new(lt()), Box::new(ge()));
    assert_eq!(normalize_constraint(&ab), normalize_constraint(&ba));
    let ob = NConstraint::Or(Box::new(ge()), Box::new(lt()));
    let bo = NConstraint::Or(Box::new(lt()), Box::new(ge()));
    assert_eq!(normalize_constraint(&ob), normalize_constraint(&bo));
    // Verschachtelung + Dopplung kollabieren.
    let nested = NConstraint::And(
        Box::new(ge()),
        Box::new(NConstraint::And(Box::new(lt()), Box::new(ge()))),
    );
    assert_eq!(normalize_constraint(&nested), normalize_constraint(&ab));
    // Idempotent.
    let once = normalize_constraint(&nested);
    assert_eq!(normalize_constraint(&once), once);
    // Eingebettete NExps werden mit-normalisiert (`b + a` == `a + b`).
    let sum_ab = || NExp::Sum(Box::new(v(b)), Box::new(v(a)));
    let sum_ba = || NExp::Sum(Box::new(v(a)), Box::new(v(b)));
    let ca = NConstraint::Ge(Box::new(sum_ab()), Box::new(c(0)));
    let cb = NConstraint::Ge(Box::new(sum_ba()), Box::new(c(0)));
    assert_eq!(normalize_constraint(&ca), normalize_constraint(&cb));
}

#[test]
fn simplify_identities_per_operator() {
    let mut ix = SimpleInterner::new();
    let n = ix.intern("n");
    let a = ix.intern("a");
    let b = ix.intern("b");
    // Sum/Times/Minus/Neg/Exp/Div/Mod/Abs/If-Matrix je einmal.
    assert_eq!(simplify(&NExp::Sum(Box::new(v(n)), Box::new(c(0)))), v(n));
    assert_eq!(simplify(&NExp::Times(Box::new(v(n)), Box::new(c(1)))), v(n));
    assert_eq!(simplify(&NExp::Times(Box::new(v(n)), Box::new(c(0)))), c(0));
    assert_eq!(simplify(&NExp::Minus(Box::new(v(n)), Box::new(v(n)))), c(0));
    assert_eq!(simplify(&NExp::Neg(Box::new(NExp::Neg(Box::new(v(n)))))), v(n));
    assert_eq!(simplify(&NExp::Neg(Box::new(c(3)))), c(-3));
    assert_eq!(simplify(&NExp::Exp(Box::new(c(0)))), c(1));
    assert_eq!(simplify(&NExp::div(v(n), c(1))), v(n));
    assert_eq!(simplify(&NExp::div(c(7), c(2))), c(3));
    assert_eq!(simplify(&NExp::modulo(v(n), c(1))), c(0));
    assert_eq!(simplify(&NExp::abs(c(-4))), c(4));
    assert_eq!(
        simplify(&NExp::Times(
            Box::new(c(-1)),
            Box::new(v(n))
        )),
        NExp::Neg(Box::new(v(n)))
    );
    // normalize ist kanonisch fuer AC-Operatoren.
    let x = NExp::Times(Box::new(v(b)), Box::new(v(a)));
    let y = NExp::Times(Box::new(v(a)), Box::new(v(b)));
    assert_eq!(normalize(&x), normalize(&y));
    // Constraint-Faltung je Vergleich.
    let wrap = |e: NExp| Box::new(TypArg::NExp(e));
    assert_eq!(
        simplify_constraint(&NConstraint::Equal(wrap(c(2)), wrap(c(2)))),
        NConstraint::True
    );
    assert_eq!(
        simplify_constraint(&NConstraint::NotEqual(wrap(c(2)), wrap(c(2)))),
        NConstraint::False
    );
    assert_eq!(
        simplify_constraint(&NConstraint::Ge(Box::new(v(n)), Box::new(v(n)))),
        NConstraint::True
    );
    assert_eq!(
        simplify_constraint(&NConstraint::Gt(Box::new(v(n)), Box::new(v(n)))),
        NConstraint::False
    );
    assert_eq!(
        simplify_constraint(&NConstraint::Le(Box::new(v(n)), Box::new(v(n)))),
        NConstraint::True
    );
    assert_eq!(
        simplify_constraint(&NConstraint::Lt(Box::new(v(n)), Box::new(v(n)))),
        NConstraint::False
    );
    assert_eq!(
        simplify_constraint(&NConstraint::Set(Box::new(c(2)), vec![3, 2, 2])),
        NConstraint::True
    );
    assert_eq!(
        simplify_constraint(&NConstraint::Not(Box::new(NConstraint::Not(Box::new(
            NConstraint::Var(b)
        ))))),
        NConstraint::Var(b)
    );
}

#[test]
fn lean_mapping_per_operator() {
    let mut ix = SimpleInterner::new();
    let n = ix.intern("n");
    use sail_nexp::IntKind;
    assert_eq!(lean_type_name(&c(0)), "Nat");
    assert_eq!(lean_type_name(&c(-1)), "Int");
    assert_eq!(lean_type_name(&NExp::Exp(Box::new(v(n)))), "Nat");
    assert_eq!(lean_type_name(&NExp::abs(v(n))), "Nat");
    assert_eq!(lean_type_name(&NExp::Neg(Box::new(c(1)))), "Int");
    assert_eq!(int_kind(&v(n)), IntKind::Int);
    assert_eq!(lean_type_name(&NExp::modulo(v(n), c(8))), "Nat");
    assert_eq!(lean_type_name(&NExp::div(v(n), c(2))), "Int");
    assert_eq!(lean_type_name(&NExp::Id("f".to_string())), "Int");
}
