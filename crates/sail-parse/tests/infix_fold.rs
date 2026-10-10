//! Infix-Faltungs-Tests (Lane 109, Code ohne Run per E3).
//!
//! Prüft [`sail_parse::infix::fold_chain`] gegen die aus
//! `sail/src/lib/infix_parser.mly` übernommene Präzedenz-Tabelle:
//! eingebaute Stufen/Assoziativitäten, benutzerdefinierte Fixities,
//! Vergleichsketten-Desugar (`&`), Präfix-Bindung und Fehlerfälle
//! (unbekannte Fixity, Nicht-Mischbarkeit). Blätter sind `String`s —
//! die Grammatik-`InfixChain` wird direkt konstruiert, kein Parsen nötig.

use sail_grammar::ast::{Def, DefAux, FixAssoc, InfixChain};
use sail_parse::infix::{Assoc, FixityEnv, Folded, Side, env_from_defs, fold_chain};

fn chain(head: &str, tail: &[(&str, &str)]) -> InfixChain<String> {
    InfixChain {
        prefixes: vec![],
        head: head.to_owned(),
        tail: tail
            .iter()
            .map(|(op, rhs)| ((*op).to_owned(), vec![], (*rhs).to_owned()))
            .collect(),
    }
}

fn leaf(s: &str) -> Folded<String> {
    Folded::Leaf(s.to_owned())
}

fn infix(op: &str, l: Folded<String>, r: Folded<String>) -> Folded<String> {
    Folded::Infix {
        op: op.to_owned(),
        lhs: Box::new(l),
        rhs: Box::new(r),
    }
}

fn prefix(op: &str, a: Folded<String>) -> Folded<String> {
    Folded::Prefix {
        op: op.to_owned(),
        arg: Box::new(a),
    }
}

fn env(entries: &[(&str, Assoc, u8)]) -> FixityEnv {
    let mut e = FixityEnv::new();
    for &(op, assoc, level) in entries {
        e.insert(op, assoc, level).unwrap();
    }
    e
}

fn empty() -> FixityEnv {
    FixityEnv::new()
}

// ---------------------------------------------------------------------------
// Eingebaute Präzedenz: `*` bindet fester als `+`
// ---------------------------------------------------------------------------

#[test]
fn add_mul_precedence_typ() {
    let got = fold_chain(&chain("a", &[("+", "b"), ("*", "c")]), Side::Typ, &empty()).unwrap();
    assert_eq!(got, infix("+", leaf("a"), infix("*", leaf("b"), leaf("c"))));
}

#[test]
fn add_mul_precedence_exp() {
    let got = fold_chain(&chain("a", &[("+", "b"), ("*", "c")]), Side::Exp, &empty()).unwrap();
    assert_eq!(got, infix("+", leaf("a"), infix("*", leaf("b"), leaf("c"))));
}

#[test]
fn mul_add_precedence() {
    let got = fold_chain(&chain("a", &[("*", "b"), ("+", "c")]), Side::Exp, &empty()).unwrap();
    assert_eq!(got, infix("+", infix("*", leaf("a"), leaf("b")), leaf("c")));
}

#[test]
fn minus_left_assoc() {
    // `a - b - c` = `(a - b) - c` (typ6l, links).
    let got = fold_chain(&chain("a", &[("-", "b"), ("-", "c")]), Side::Exp, &empty()).unwrap();
    assert_eq!(got, infix("-", infix("-", leaf("a"), leaf("b")), leaf("c")));
}

// ---------------------------------------------------------------------------
// Benutzerdefinierte Fixities
// ---------------------------------------------------------------------------

#[test]
fn user_right_assoc_nests_right() {
    let e = env(&[("++", Assoc::Right, 7)]);
    let got = fold_chain(&chain("a", &[("++", "b"), ("++", "c")]), Side::Exp, &e).unwrap();
    assert_eq!(got, infix("++", leaf("a"), infix("++", leaf("b"), leaf("c"))));
}

#[test]
fn user_left_assoc_nests_left() {
    let e = env(&[("++", Assoc::Left, 7)]);
    let got = fold_chain(&chain("a", &[("++", "b"), ("++", "c")]), Side::Exp, &e).unwrap();
    assert_eq!(got, infix("++", infix("++", leaf("a"), leaf("b")), leaf("c")));
}

#[test]
fn user_nonassoc_rejects_chain() {
    let e = env(&[("@@", Assoc::Non, 5)]);
    let err = fold_chain(&chain("a", &[("@@", "b"), ("@@", "c")]), Side::Exp, &e).unwrap_err();
    assert!(err.to_string().contains("parentheses"), "got: {err}");
}

#[test]
fn level0_binds_loosest() {
    let e = env(&[("|||", Assoc::Left, 0)]);
    let got =
        fold_chain(&chain("a", &[("|||", "b"), ("+", "c")]), Side::Exp, &e).unwrap();
    assert_eq!(got, infix("|||", leaf("a"), infix("+", leaf("b"), leaf("c"))));
}

#[test]
fn caret_is_user_op() {
    // `^` ist kein Built-in (nur per `infixr 6 ^` deklariert).
    let e = env(&[("^", Assoc::Right, 6)]);
    let got = fold_chain(&chain("a", &[("^", "b"), ("^", "c")]), Side::Exp, &e).unwrap();
    assert_eq!(got, infix("^", leaf("a"), infix("^", leaf("b"), leaf("c"))));
}

// ---------------------------------------------------------------------------
// Nicht-Mischbarkeit auf gleicher Stufe (getrennte `.mly`-Arme)
// ---------------------------------------------------------------------------

#[test]
fn builtin_star_rejects_user_nonassoc_same_level() {
    let e = env(&[("%%", Assoc::Non, 7)]);
    assert!(fold_chain(&chain("a", &[("*", "b"), ("%%", "c")]), Side::Exp, &e).is_err());
    assert!(fold_chain(&chain("a", &[("%%", "b"), ("*", "c")]), Side::Exp, &e).is_err());
}

#[test]
fn builtin_plus_mixes_with_user_left_same_level() {
    // `.mly` teilt sich `typ6l`/`exp6l` zwischen `+` und `Op6l`.
    let e = env(&[("++", Assoc::Left, 6)]);
    let got = fold_chain(&chain("a", &[("+", "b"), ("++", "c")]), Side::Exp, &e).unwrap();
    assert_eq!(got, infix("++", infix("+", leaf("a"), leaf("b")), leaf("c")));
}

#[test]
fn builtin_plus_rejects_user_right_same_level() {
    // `Op6r` hat eigene Arme (`typ7 Op6r typ6r`) — kein Mischen mit `+`.
    let e = env(&[("++", Assoc::Right, 6)]);
    assert!(fold_chain(&chain("a", &[("+", "b"), ("++", "c")]), Side::Exp, &e).is_err());
}

#[test]
fn at_mixes_with_user_right_same_level() {
    // `.mly` `exp5r`: `At` und `Op5r` teilen sich den Arm.
    let e = env(&[("++", Assoc::Right, 5)]);
    let got = fold_chain(&chain("a", &[("@", "b"), ("++", "c")]), Side::Exp, &e).unwrap();
    assert_eq!(got, infix("@", leaf("a"), infix("++", leaf("b"), leaf("c"))));
}

#[test]
fn at_chains_right() {
    let got = fold_chain(&chain("a", &[("@", "b"), ("@", "c")]), Side::Exp, &empty()).unwrap();
    assert_eq!(got, infix("@", leaf("a"), infix("@", leaf("b"), leaf("c"))));
}

#[test]
fn cons_chains_right() {
    let got =
        fold_chain(&chain("a", &[("::", "b"), ("::", "c")]), Side::Exp, &empty()).unwrap();
    assert_eq!(got, infix("::", leaf("a"), infix("::", leaf("b"), leaf("c"))));
}

// ---------------------------------------------------------------------------
// Vergleichsketten
// ---------------------------------------------------------------------------

#[test]
fn typ_lchain_desugars_via_and() {
    // `desugar_lchain`: `a < b <= c` = `(a<b) & (b<=c)` (Mitte dupliziert).
    let got =
        fold_chain(&chain("a", &[("<", "b"), ("<=", "c")]), Side::Typ, &empty()).unwrap();
    assert_eq!(
        got,
        infix(
            "&",
            infix("<", leaf("a"), leaf("b")),
            infix("<=", leaf("b"), leaf("c")),
        )
    );
}

#[test]
fn typ_rchain_desugars_via_and() {
    let got =
        fold_chain(&chain("a", &[(">=", "b"), (">", "c")]), Side::Typ, &empty()).unwrap();
    assert_eq!(
        got,
        infix(
            "&",
            infix(">=", leaf("a"), leaf("b")),
            infix(">", leaf("b"), leaf("c")),
        )
    );
}

#[test]
fn typ_lchain_three_nests_and_right() {
    let got = fold_chain(
        &chain("a", &[("<", "b"), ("<=", "c"), ("<", "d")]),
        Side::Typ,
        &empty(),
    )
    .unwrap();
    assert_eq!(
        got,
        infix(
            "&",
            infix("<", leaf("a"), leaf("b")),
            infix(
                "&",
                infix("<=", leaf("b"), leaf("c")),
                infix("<", leaf("c"), leaf("d")),
            ),
        )
    );
}

#[test]
fn typ_mixed_lt_gt_rejects() {
    // lchain/rchain sind disjunkt — Mischen braucht Klammern.
    assert!(fold_chain(&chain("a", &[("<", "b"), (">", "c")]), Side::Typ, &empty()).is_err());
}

#[test]
fn exp_comparison_does_not_chain() {
    // `.mly` `exp4`: `exp5 Lt exp5` genau einmal.
    assert!(fold_chain(&chain("a", &[("<", "b"), ("<", "c")]), Side::Exp, &empty()).is_err());
}

// ---------------------------------------------------------------------------
// Seitenspezifische Operatoren
// ---------------------------------------------------------------------------

#[test]
fn in_is_typ_only() {
    let got = fold_chain(&chain("a", &[("in", "b")]), Side::Typ, &empty()).unwrap();
    assert_eq!(got, infix("in", leaf("a"), leaf("b")));
    assert!(fold_chain(&chain("a", &[("in", "b")]), Side::Exp, &empty()).is_err());
}

#[test]
fn at_cons_are_exp_only() {
    assert!(fold_chain(&chain("a", &[("@", "b")]), Side::Typ, &empty()).is_err());
    assert!(fold_chain(&chain("a", &[("::", "b")]), Side::Typ, &empty()).is_err());
}

// ---------------------------------------------------------------------------
// Präfixe
// ---------------------------------------------------------------------------

#[test]
fn negate_in_rhs() {
    let c = InfixChain {
        prefixes: vec![],
        head: "a".to_owned(),
        tail: vec![("*".to_owned(), vec!["negate".to_owned()], "b".to_owned())],
    };
    let got = fold_chain(&c, Side::Exp, &empty()).unwrap();
    assert_eq!(got, infix("*", leaf("a"), prefix("negate", leaf("b"))));
}

#[test]
fn negate_binds_looser_than_level9() {
    // `.mly` `Minus typ9`: `-a ++ b` = `-(a ++ b)` für Stufe-9-`++`.
    let e = env(&[("++", Assoc::Left, 9)]);
    let c = InfixChain {
        prefixes: vec!["negate".to_owned()],
        head: "a".to_owned(),
        tail: vec![("++".to_owned(), vec![], "b".to_owned())],
    };
    let got = fold_chain(&c, Side::Exp, &e).unwrap();
    assert_eq!(got, prefix("negate", infix("++", leaf("a"), leaf("b"))));
}

#[test]
fn double_prefix_rejects() {
    // `.mly` `exp8`: `Minus exp9`, `exp9` kennt kein Präfix.
    let c = InfixChain {
        prefixes: vec!["negate".to_owned(), "negate".to_owned()],
        head: "x".to_owned(),
        tail: vec![],
    };
    assert!(fold_chain(&c, Side::Exp, &empty()).is_err());
}

#[test]
fn prefix_needs_atomic_for_deref() {
    // `.mly`: `Star atomic_exp` — `*p + x` = `(*p) + x`.
    let c = InfixChain {
        prefixes: vec!["__deref".to_owned()],
        head: "p".to_owned(),
        tail: vec![("+".to_owned(), vec![], "x".to_owned())],
    };
    let got = fold_chain(&c, Side::Exp, &empty()).unwrap();
    assert_eq!(got, infix("+", prefix("__deref", leaf("p")), leaf("x")));
}

#[test]
fn deref_is_exp_only() {
    let c = InfixChain {
        prefixes: vec!["__deref".to_owned()],
        head: "p".to_owned(),
        tail: vec![],
    };
    assert!(fold_chain(&c, Side::Typ, &empty()).is_err());
}

#[test]
fn pow2_prefix() {
    let c = InfixChain {
        prefixes: vec!["pow2".to_owned()],
        head: "n".to_owned(),
        tail: vec![],
    };
    let got = fold_chain(&c, Side::Typ, &empty()).unwrap();
    assert_eq!(got, prefix("pow2", leaf("n")));
}

// ---------------------------------------------------------------------------
// Umgebung, Fehler, Randfälle
// ---------------------------------------------------------------------------

#[test]
fn undeclared_fixity_errors() {
    let err = fold_chain(&chain("a", &[("???", "b")]), Side::Exp, &empty()).unwrap_err();
    assert!(err.to_string().contains("Undeclared fixity"), "got: {err}");
}

#[test]
fn builtin_shadows_user_table() {
    // Der OCaml-`supplier` matcht `+` vor dem Tabellen-Lookup.
    let e = env(&[("+", Assoc::Right, 0)]);
    let got = fold_chain(&chain("a", &[("+", "b"), ("+", "c")]), Side::Exp, &e).unwrap();
    assert_eq!(got, infix("+", infix("+", leaf("a"), leaf("b")), leaf("c")));
}

#[test]
fn fixity_level_range_checked() {
    let mut e = FixityEnv::new();
    assert!(e.insert("++", Assoc::Left, 9).is_ok());
    assert!(e.insert("++", Assoc::Left, 10).is_err());
}

#[test]
fn single_atom_folds_to_leaf() {
    assert_eq!(fold_chain(&chain("x", &[]), Side::Exp, &empty()).unwrap(), leaf("x"));
}

#[test]
fn env_from_defs_collects_fixities_through_wrappers() {
    let defs = vec![
        Def::Aux(DefAux::Fixity(FixAssoc::InfixR, 5, "++".to_owned())),
        Def::Private(Box::new(Def::Aux(DefAux::Fixity(
            FixAssoc::InfixL,
            6,
            "+++ ".to_owned(),
        )))),
    ];
    let env = env_from_defs(&defs).unwrap();
    let got = fold_chain(&chain("a", &[("++", "b"), ("++", "c")]), Side::Exp, &env).unwrap();
    assert_eq!(got, infix("++", leaf("a"), infix("++", leaf("b"), leaf("c"))));
    // `+++ ` (mit Leerzeichen) ist ein anderer Name als `+++`.
    assert!(env.get("+++").is_none());
}
