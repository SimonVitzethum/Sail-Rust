#![forbid(unsafe_code)]
//! Lane 108 (`ncapp-args`): `NC_app` + `NC_equal`/`NC_not_equal` ueber
//! `typ_arg`-Sorten (`A_nexp`/`A_typ`/`A_bool`, OCaml `Ast.ml`).
//!
//! Reine Code-Tests (E3: nicht ausfuehren) — Typen, Konverter und
//! `eval`/`simplify`-Anbindung fuer Typ-/Bool-Argumente.

use sail_ast::{
    ConstraintKind, Interner as _, KOpt, Kind, Name, NExpKind, Node, SimpleInterner, TypArgKind,
    TypKind,
};
use sail_nexp::{
    constraint_from_ast, constraint_to_ast, normalize_constraint, normalize_typarg,
    simplify_constraint, simplify_constraint_cow, simplify_typarg, typarg_from_ast, typarg_to_ast,
    Kid, NConstraint, NExp, NKind, NQuant, NTyp, TypArg,
};
use std::borrow::Cow;
use std::collections::BTreeMap;

fn kids() -> (SimpleInterner, Kid, Kid) {
    let mut ix = SimpleInterner::new();
    let n = ix.intern("n");
    let m = ix.intern("m");
    (ix, n, m)
}

fn v(k: Kid) -> NExp {
    NExp::Var(k)
}
fn c(i: i128) -> NExp {
    NExp::Constant(i)
}
fn ne(e: NExp) -> Box<TypArg> {
    Box::new(TypArg::NExp(e))
}
fn ty(t: NTyp) -> Box<TypArg> {
    Box::new(TypArg::Typ(t))
}
fn bo(c: NConstraint) -> Box<TypArg> {
    Box::new(TypArg::Bool(Box::new(c)))
}
fn env(ix: &mut SimpleInterner, pairs: &[(&str, i128)]) -> BTreeMap<Kid, i128> {
    pairs.iter().map(|(k, x)| (ix.intern(k), *x)).collect()
}
fn ast_name(ix: &mut SimpleInterner, text: &str) -> Name {
    Name::dummy(ix.intern(text))
}

#[test]
fn typarg_sorts_are_representable() {
    let (_ix, n, m) = kids();
    // Alle drei Sorten als NC_app-Argumente.
    let app = NConstraint::App(
        "p".to_string(),
        vec![
            TypArg::NExp(v(n)),
            TypArg::Typ(NTyp::Var(m)),
            TypArg::Bool(Box::new(NConstraint::True)),
        ],
    );
    assert_eq!(app.vars(), vec![n, m]);
    // NC_equal/not_equal ueber jeder Sorte.
    let eq_typ = NConstraint::Equal(ty(NTyp::Var(n)), ty(NTyp::Var(n)));
    assert_eq!(eq_typ.eval(&BTreeMap::new()), Some(true));
    let neq_bool = NConstraint::NotEqual(bo(NConstraint::True), bo(NConstraint::False));
    assert_eq!(neq_bool.eval(&BTreeMap::new()), Some(true));
}

#[test]
fn typarg_converters_cover_all_sorts() {
    let mut ix = SimpleInterner::new();
    let n = ix.intern("n");
    // A_nexp.
    let a = TypArg::NExp(NExp::Var(n));
    assert_eq!(typarg_from_ast(&typarg_to_ast(&a, &mut ix), &ix), Some(a));
    // A_typ: Struktur bleibt erhalten (App + Var + Tuple).
    let t = TypArg::Typ(NTyp::App(
        "pair".to_string(),
        vec![
            TypArg::NExp(NExp::Var(n)),
            TypArg::Typ(NTyp::Tuple(vec![NTyp::Var(n), NTyp::Id("bool".to_string())])),
        ],
    ));
    assert_eq!(typarg_from_ast(&typarg_to_ast(&t, &mut ix), &ix), Some(t));
    // A_bool.
    let b = TypArg::Bool(Box::new(NConstraint::Ge(
        Box::new(NExp::Var(n)),
        Box::new(NExp::Constant(0)),
    )));
    assert_eq!(typarg_from_ast(&typarg_to_ast(&b, &mut ix), &ix), Some(b));
    // Existential mit Bindern (Shadowing-relevant).
    let e = NTyp::Exist(
        vec![NQuant { kind: NKind::Int, kid: n }],
        Box::new(NConstraint::Ge(Box::new(NExp::Var(n)), Box::new(NExp::Constant(0)))),
        Box::new(NTyp::Var(n)),
    );
    assert_eq!(
        sail_nexp::ntyp_from_ast(&sail_nexp::ntyp_to_ast(&e, &mut ix), &ix),
        Some(e)
    );
}

#[test]
fn constraint_converters_cover_typ_bool_app_and_equal() {
    let mut ix = SimpleInterner::new();
    let n = ix.intern("n");
    // NC_equal ueber A_typ-Paar.
    let eq = NConstraint::Equal(ty(NTyp::Var(n)), ty(NTyp::Var(n)));
    let ast = constraint_to_ast(&eq, &mut ix);
    assert!(matches!(ast.kind, ConstraintKind::Equal(_, _)));
    assert_eq!(constraint_from_ast(&ast, &ix), Some(eq));
    // NC_app mit gemischten Sorten.
    let app = NConstraint::App(
        "ok".to_string(),
        vec![
            TypArg::NExp(c(1)),
            TypArg::Typ(NTyp::Unknown),
            TypArg::Bool(Box::new(NConstraint::False)),
        ],
    );
    let ast = constraint_to_ast(&app, &mut ix);
    assert!(matches!(ast.kind, ConstraintKind::App { .. }));
    assert_eq!(constraint_from_ast(&ast, &ix), Some(app));
    // NC_set-Members als Dezimal-Strings.
    let set = NConstraint::Set(Box::new(v(n)), vec![3, 1, 3]);
    let ast = constraint_to_ast(&set, &mut ix);
    assert!(matches!(ast.kind, ConstraintKind::Set { .. }));
    assert_eq!(constraint_from_ast(&ast, &ix), Some(set));
    // Unparsbare Konstante => None (kein stilles Raten).
    let bad = Node::dummy(NExpKind::Constant("12x".to_string()));
    let bad_arg = Node::dummy(TypArgKind::NExp(bad));
    let bad_eq = Node::dummy(ConstraintKind::Equal(
        Box::new(bad_arg.clone()),
        Box::new(bad_arg),
    ));
    assert_eq!(constraint_from_ast(&bad_eq, &ix), None);
    // Fn-/Bidir-/Exist-Typen mappen strukturell (kein None).
    let f = Node::dummy(TypKind::Fn {
        args: vec![Node::dummy(TypKind::Id(ast_name(&mut ix, "int")))],
        ret: Box::new(Node::dummy(TypKind::Var(n))),
    });
    assert!(sail_nexp::ntyp_from_ast(&f, &ix).is_some());
    let q = Node::dummy(TypKind::Exist {
        binders: vec![KOpt { kind: Kind::Bool, kid: n }],
        constraint: Box::new(Node::dummy(ConstraintKind::True)),
        typ: Box::new(Node::dummy(TypKind::Unknown)),
    });
    assert!(sail_nexp::ntyp_from_ast(&q, &ix).is_some());
}

#[test]
fn equal_not_equal_eval_per_sort() {
    let (mut ix, n, _m) = kids();
    let e = env(&mut ix, &[("n", 3)]);
    // (NExp, NExp): Vergleich wie bisher.
    assert_eq!(
        NConstraint::Equal(ne(v(n)), ne(c(3))).eval(&e),
        Some(true)
    );
    assert_eq!(
        NConstraint::NotEqual(ne(v(n)), ne(c(3))).eval(&e),
        Some(false)
    );
    // Syntaktisch gleiche Args sind gleich (auch ungebunden).
    let open = NConstraint::Equal(ne(v(n)), ne(v(n)));
    assert_eq!(open.eval(&BTreeMap::new()), Some(true));
    // (Bool, Bool): rekursive Valuation.
    let beq = NConstraint::Equal(bo(NConstraint::True), bo(NConstraint::False));
    assert_eq!(beq.eval(&e), Some(false));
    assert_eq!(
        NConstraint::NotEqual(bo(NConstraint::True), bo(NConstraint::False)).eval(&e),
        Some(true)
    );
    // Gemischte Sorten sind nie gleich.
    assert_eq!(
        NConstraint::Equal(ne(c(1)), bo(NConstraint::True)).eval(&e),
        Some(false)
    );
    assert_eq!(
        NConstraint::NotEqual(ne(c(1)), ty(NTyp::Unknown)).eval(&e),
        Some(true)
    );
    // Verschiedene Typen: konservativ None; App bleibt uninterpretiert.
    assert_eq!(
        NConstraint::Equal(ty(NTyp::Unknown), ty(NTyp::Id("bool".to_string()))).eval(&e),
        None
    );
    let app = NConstraint::App("p".to_string(), vec![TypArg::NExp(c(1))]);
    assert_eq!(app.eval(&e), None);
}

#[test]
fn simplify_folds_per_sort() {
    let (_ix, n, _m) = kids();
    // a == a -> True (jede Sorte), a != a -> False.
    assert_eq!(
        simplify_constraint(&NConstraint::Equal(ty(NTyp::Unknown), ty(NTyp::Unknown))),
        NConstraint::True
    );
    assert_eq!(
        simplify_constraint(&NConstraint::NotEqual(
            bo(NConstraint::True),
            bo(NConstraint::True)
        )),
        NConstraint::False
    );
    // Gemischte Sorten falten sofort.
    assert_eq!(
        simplify_constraint(&NConstraint::Equal(ne(c(1)), ty(NTyp::Unknown))),
        NConstraint::False
    );
    assert_eq!(
        simplify_constraint(&NConstraint::NotEqual(ne(c(1)), bo(NConstraint::True))),
        NConstraint::True
    );
    // NExp-Konstanten wie bisher; Bool-Literale falten.
    assert_eq!(
        simplify_constraint(&NConstraint::Equal(ne(c(2)), ne(c(3)))),
        NConstraint::False
    );
    assert_eq!(
        simplify_constraint(&NConstraint::Equal(bo(NConstraint::True), bo(NConstraint::False))),
        NConstraint::False
    );
    // NC_app faltet nur Argumente, nie das Praedikat.
    let app = NConstraint::App(
        "p".to_string(),
        vec![
            TypArg::NExp(NExp::Sum(Box::new(c(1)), Box::new(c(2)))),
            TypArg::Bool(Box::new(NConstraint::And(
                Box::new(NConstraint::True),
                Box::new(NConstraint::False),
            ))),
            TypArg::Typ(NTyp::Unknown),
        ],
    );
    assert_eq!(
        simplify_constraint(&app),
        NConstraint::App(
            "p".to_string(),
            vec![
                TypArg::NExp(c(3)),
                TypArg::Bool(Box::new(NConstraint::False)),
                TypArg::Typ(NTyp::Unknown),
            ]
        )
    );
    // Unveraendert wird geborgt (Borrowed-Vertrag).
    let stable = NConstraint::App(
        "p".to_string(),
        vec![TypArg::NExp(v(n)), TypArg::Typ(NTyp::Var(n))],
    );
    assert!(matches!(simplify_constraint_cow(&stable), Cow::Borrowed(_)));
    // simplify_typarg deckt jede Sorte ab.
    assert_eq!(simplify_typarg(&TypArg::Typ(NTyp::Unknown)), TypArg::Typ(NTyp::Unknown));
    assert_eq!(normalize_typarg(&TypArg::NExp(v(n))), TypArg::NExp(v(n)));
}

#[test]
fn subst_and_vars_traverse_typ_and_bool() {
    let (_ix, n, m) = kids();
    // vars in Erstauftretens-Reihenfolge ueber alle Sorten.
    let app = NConstraint::App(
        "p".to_string(),
        vec![
            TypArg::Typ(NTyp::App("f".to_string(), vec![TypArg::NExp(NExp::Var(m))])),
            TypArg::Bool(Box::new(NConstraint::Var(n))),
            TypArg::NExp(NExp::Var(m)),
        ],
    );
    assert_eq!(app.vars(), vec![m, n]);
    // subst trifft NExp- und Bool-Positionen (inkl. Bool-Faltung 0 -> False).
    let mut s = BTreeMap::new();
    s.insert(m, c(9));
    s.insert(n, c(0));
    assert_eq!(
        app.subst(&s),
        NConstraint::App(
            "p".to_string(),
            vec![
                TypArg::Typ(NTyp::App("f".to_string(), vec![TypArg::NExp(c(9))])),
                TypArg::Bool(Box::new(NConstraint::False)),
                TypArg::NExp(c(9)),
            ]
        )
    );
    // Exist-Binder schatten: gebundenes n bleibt, freies m wird ersetzt.
    let ex = NTyp::Exist(
        vec![NQuant { kind: NKind::Int, kid: n }],
        Box::new(NConstraint::Ge(Box::new(v(n)), Box::new(v(m)))),
        Box::new(NTyp::Tuple(vec![NTyp::Var(n), NTyp::Var(m)])),
    );
    let mut s2 = BTreeMap::new();
    s2.insert(n, c(1));
    s2.insert(m, c(2));
    assert_eq!(
        ex.subst(&s2),
        NTyp::Exist(
            vec![NQuant { kind: NKind::Int, kid: n }],
            Box::new(NConstraint::Ge(Box::new(v(n)), Box::new(c(2)))),
            Box::new(NTyp::Tuple(vec![NTyp::Var(n), NTyp::Var(m)])),
        )
    );
    // Gebundene Vorkommen sind keine freien Variablen.
    assert_eq!(ex.vars(), vec![m]);
    // Unveraenderte Typ-Args werden geborgt.
    let t = TypArg::Typ(NTyp::Var(n));
    let empty: BTreeMap<Kid, NExp> = BTreeMap::new();
    assert!(matches!(t.subst_cow(&empty), Cow::Borrowed(_)));
}

#[test]
fn normalize_is_canonical_with_typargs() {
    let (_ix, n, m) = kids();
    // And-Reihenfolge + eingebettete TypArg-Normalisierung sind kanonisch.
    let ge = || NConstraint::Ge(Box::new(v(n)), Box::new(c(0)));
    let app = || {
        NConstraint::App(
            "p".to_string(),
            vec![TypArg::NExp(NExp::Sum(Box::new(v(m)), Box::new(v(n))))],
        )
    };
    let ab = NConstraint::And(Box::new(ge()), Box::new(app()));
    let ba = NConstraint::And(Box::new(app()), Box::new(ge()));
    assert_eq!(normalize_constraint(&ab), normalize_constraint(&ba));
    // Fixpunkt.
    let once = normalize_constraint(&ab);
    assert_eq!(normalize_constraint(&once), once);
    // Gemischte Equal-Args normalisieren eingebettete NExps mit.
    let e1 = NConstraint::Equal(
        ne(NExp::Sum(Box::new(v(m)), Box::new(v(n)))),
        ty(NTyp::Unknown),
    );
    let e2 = NConstraint::Equal(
        ne(NExp::Sum(Box::new(v(n)), Box::new(v(m)))),
        ty(NTyp::Unknown),
    );
    assert_eq!(normalize_constraint(&e1), normalize_constraint(&e2));
}
