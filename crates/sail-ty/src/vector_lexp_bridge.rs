#![forbid(unsafe_code)]
//! Vektor-LHS-Brücke (Lane 41, exklusiv neu).
//!
//! Adapter zwischen dem kanonischen Checker (`canonical_body.rs`: `Type` mit
//! Substitutionen über `sail_ast::Typ`) und der Lane-07/30-Vektorwelt
//! (`lexp_bounds` / `vector_lexp`: [`MiniTyp`] / [`Nexp`]).
//!
//! - [`index_nexp_from_exp`]: synthetisiert den Index-[`Nexp`] aus einem
//!   kanonischen [`Exp`](sail_ast::Exp). Nur Literale (`42`), Bezeichner
//!   (`i`) und transparente Annotationen (`(i : int)`) werden abgebildet;
//!   alles andere (Applikationen, Vektoren, ...) ist
//!   [`Unsupported`](crate::error::TypeErrorKind::Unsupported) — Lücken
//!   werden nie geraten.
//! - [`mini_typ_of_canonical`]: hebt einen kanonischen Typknoten nach
//!   [`MiniTyp`]. Unbekannte Formen (`Var`, `Bidir`, `Exist`, ...) sind
//!   `Unsupported`. Eine Ausnahme ist dokumentiert, kein Raten:
//!   `atom(..)` ist exakt das Bild von `open_value` (kanonischer Checker
//!   öffnet `int` zu `atom(__value)`), also `int`-artig und wird zu
//!   [`MiniTyp::Int`].
//! - [`ast_of_mini_typ`]: hebt ein [`MiniTyp`]-Ergebnis
//!   (Element-/Slice-Typ aus `lexp_bounds`) nach [`Typ`](sail_ast::Typ).
//!   Total: jede `MiniTyp`-Form hat ein kanonisches Bild
//!   (`conv::typ_to_ast`); hier wird nichts bewiesen und nichts geraten.
//! - [`vector_root_of_canonical`]: expandierte Vektor-Wurzel aus einem
//!   kanonischen Typknoten (dünner Wrapper über
//!   [`vector_lexp::vector_root_of_typ`](crate::vector_lexp::vector_root_of_typ);
//!   der Aufrufer übergibt den bereits per `resolve` expandierten Knoten.
//! - [`mini_order_of_ast`]: reiner Positions-Mapping von
//!   [`Order`](sail_ast::Order) (kein Fallback; eine fehlende
//!   Default-Order meldet der Hook, nicht diese Brücke).
//!
//! Beweisverpflichtungen laufen nie durch diese Brücke: Der Hook
//! (`canonical_body.rs`, siehe `patch-vector-lexp.diff`) prüft die
//! `Mini`-Seite (`check_index_typ`), beweist die Obligationen
//! (`index_obligation` / `slice_obligation`) über `program.prove` und hebt
//! erst danach das Ergebnis. Reihenfolge ist load-bearing:
//! Wurzel expandieren, Index inferieren, Bounds zuerst, dann RHS;
//! LExp-Fehler gewinnen immer (`check_assign`-Priorität via `?` vor dem
//! RHS-Task).

use crate::error::{Span, TypeError, TypeErrorKind, TypeResult};
use crate::lexp_bounds::VectorRoot;
use crate::nexp::{MiniOrder, MiniTyp, Nexp};
use sail_ast::{Exp, ExpKind, Interner, LitKind, Order as AstOrder, Typ};

fn unsupported(span: Span, message: &str) -> TypeError {
    TypeError {
        span,
        kind: TypeErrorKind::Unsupported(message.to_owned()),
    }
}

/// Synthetisiert den Index-[`Nexp`] aus einem kanonischen [`Exp`].
///
/// Abgebildet werden `Lit(Num)` (dezimal, `i128`-parsebar), `Id`
/// (aufgelöster Bezeichnername), transparente `Typ`-Annotationen sowie
/// reine Zahlen-Binops (`+`, `-`, `*`, unäres `-`) darüber.
/// Jede andere Form braucht Elaboration vor dem Beweis und ist
/// `Unsupported` — es wird nie ein Nexp geraten. Namen lösen über die
/// Union-Sicht auf (mehrere Tabellen); Unbekanntes bleibt `Unsupported`.
pub fn index_nexp_from_exp(exp: &Exp, it: &mut crate::vector_lexp::Union<'_>) -> TypeResult<Nexp> {
    let span = exp.span;
    match &exp.kind {
        ExpKind::Lit(lit) => match &lit.kind {
            LitKind::Num(symbol) => it
                .resolve(*symbol)
                .and_then(|text| text.parse::<i128>().ok())
                .map(Nexp::Const)
                .ok_or_else(|| {
                    unsupported(span, "vector index literal needs elaboration before proof")
                }),
            _ => Err(unsupported(
                span,
                "vector index expression needs elaboration before proof",
            )),
        },
        ExpKind::Id(name) => it
            .resolve(name.symbol)
            .map(|text| Nexp::Var(text.to_owned()))
            .ok_or_else(|| {
                unsupported(span, "vector index variable needs elaboration before proof")
            }),
        ExpKind::Typ { exp: inner, .. } => index_nexp_from_exp(inner, it),
        ExpKind::App { func, args } => {
            let op = it.resolve(func.symbol).unwrap_or("");
            match (op, args.as_slice()) {
                ("+", [a, b]) => Ok(Nexp::Add(
                    Box::new(index_nexp_from_exp(a, it)?),
                    Box::new(index_nexp_from_exp(b, it)?),
                )),
                ("-", [a, b]) => Ok(Nexp::Sub(
                    Box::new(index_nexp_from_exp(a, it)?),
                    Box::new(index_nexp_from_exp(b, it)?),
                )),
                ("*", [a, b]) => Ok(Nexp::Mul(
                    Box::new(index_nexp_from_exp(a, it)?),
                    Box::new(index_nexp_from_exp(b, it)?),
                )),
                ("negate", [a]) | ("-", [a]) => Ok(Nexp::Neg(Box::new(
                    index_nexp_from_exp(a, it)?,
                ))),
                _ => Err(unsupported(
                    span,
                    "vector index expression needs elaboration before proof",
                )),
            }
        }
        _ => Err(unsupported(
            span,
            "vector index expression needs elaboration before proof",
        )),
    }
}

/// Hebt einen kanonischen Typknoten nach [`MiniTyp`].
///
/// `None` aus `conv::typ_from_ast` (`Var`, `Bidir`, `Exist`, Fremdformen)
/// wird `Unsupported`, nie geraten. Einzige dokumentierte Normalisierung:
/// `atom(..)` ist exakt das `open_value`-Bild von `int` (der kanonische
/// Checker öffnet jeden gelesenen `int`-Wert zum Singleton), also
/// `int`-artig und damit [`MiniTyp::Int`]. `atom_bool` wird bewusst nicht
/// normalisiert (Bool-Indizes meldet `check_index_typ` als `Mismatch`).
pub fn mini_typ_of_canonical(node: &Typ, it: &impl Interner, span: Span) -> TypeResult<MiniTyp> {
    match crate::conv::typ_from_ast(node, it) {
        None => Err(unsupported(
            span,
            "vector index type needs elaboration before proof",
        )),
        Some(MiniTyp::Named { name, .. }) if name == "atom" => Ok(MiniTyp::Int),
        Some(mini) => Ok(mini),
    }
}

/// Hebt ein [`MiniTyp`]-Ergebnis (Element-/Slice-Typ) nach [`Typ`].
///
/// Total über alle `MiniTyp`-Formen (`conv::typ_to_ast`); synthetisierte
/// Knoten tragen `Span::DUMMY`-Hygiene aus `conv`. Der Hook verpackt das
/// Ergebnis danach in seinen `Type`-View (`Type::owned`).
pub fn ast_of_mini_typ(mini: &MiniTyp, it: &mut impl Interner) -> Typ {
    crate::conv::typ_to_ast(mini, it)
}

/// Extrahiert die expandierte Vektor-Wurzel aus einem kanonischen Typknoten.
///
/// Der Aufrufer übergibt den bereits per `resolve` (Synonym-Expansion)
/// aufgelösten Knoten mit seinem belegenden Interner:
/// `vector(len, elem)` / `bitvector(len)`, sonst
/// `"Invalid vector assignment l-expression"` (kein fabrizierter Typ).
pub fn vector_root_of_canonical(
    node: &Typ,
    union: &mut crate::vector_lexp::Union<'_>,
    span: Span,
) -> TypeResult<VectorRoot> {
    crate::vector_lexp::vector_root_of_typ(node, union, span)
}

/// Reine Positionsabbildung [`AstOrder`] -> [`MiniOrder`] (kein Fallback).
#[must_use]
pub fn mini_order_of_ast(order: &AstOrder) -> MiniOrder {
    crate::vector_lexp::ast_order_to_mini(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sail_ast::{NExpKind, Name, Node, SimpleInterner, TypArgKind, TypKind};

    fn mk_name(it: &mut impl Interner, text: &str) -> Name {
        Name::dummy(it.intern(text))
    }

    fn mk_lit_num(it: &mut impl Interner, value: &str) -> Exp {
        let sym = it.intern(value);
        Node::dummy(ExpKind::Lit(Node::dummy(LitKind::Num(sym))))
    }

    fn mk_id(it: &mut impl Interner, name: &str) -> Exp {
        Node::dummy(ExpKind::Id(mk_name(it, name)))
    }

    fn exp_nexp(exp: &Exp, it: &mut SimpleInterner) -> TypeResult<Nexp> {
        let mut union = crate::vector_lexp::Union::over(vec![it as &dyn Interner]);
        index_nexp_from_exp(exp, &mut union)
    }

    fn mk_int_typ(it: &mut impl Interner) -> Typ {
        Node::dummy(TypKind::Id(mk_name(it, "int")))
    }

    fn mk_atom_typ(it: &mut impl Interner) -> Typ {
        let name = mk_name(it, "atom");
        let kid = it.intern("__value");
        let arg: sail_ast::TypArg = Node::dummy(TypArgKind::NExp(Node::dummy(NExpKind::Var(kid))));
        Node::dummy(TypKind::App {
            name,
            args: vec![arg],
        })
    }

    #[test]
    fn index_synth_accepts_lit_id_and_annotation() {
        let mut it = SimpleInterner::new();
        let lit = mk_lit_num(&mut it, "42");
        assert_eq!(exp_nexp(&lit, &mut it).unwrap(), Nexp::Const(42));
        let id = mk_id(&mut it, "i");
        assert_eq!(
            exp_nexp(&id, &mut it).unwrap(),
            Nexp::Var("i".to_owned())
        );
        let annot: Exp = Node::dummy(ExpKind::Typ {
            typ: mk_int_typ(&mut it),
            exp: Box::new(mk_id(&mut it, "j")),
        });
        assert_eq!(
            exp_nexp(&annot, &mut it).unwrap(),
            Nexp::Var("j".to_owned())
        );
    }

    #[test]
    fn index_synth_rejects_everything_else_as_unsupported() {
        let mut it = SimpleInterner::new();
        // Applikation (z.B. `i + 1`): keine Elaboration, kein Raten.
        let app: Exp = Node::dummy(ExpKind::App {
            func: mk_name(&mut it, "add"),
            args: vec![mk_id(&mut it, "i")],
        });
        let err = exp_nexp(&app, &mut it).unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::Unsupported(_)));

        // Nicht-numerisches Literal.
        let tru: Exp = Node::dummy(ExpKind::Lit(Node::dummy(LitKind::True)));
        let err = exp_nexp(&tru, &mut it).unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::Unsupported(_)));

        // Block/Vektor: ebenfalls Lücke.
        let vec: Exp = Node::dummy(ExpKind::Vector(vec![mk_id(&mut it, "i")]));
        let err = exp_nexp(&vec, &mut it).unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::Unsupported(_)));
    }

    #[test]
    fn index_lit_with_unparseable_symbol_is_unsupported() {
        let mut it = SimpleInterner::new();
        // Symbol löst auf, parst aber nicht als i128.
        let lit = mk_lit_num(&mut it, "not-a-number");
        let err = exp_nexp(&lit, &mut it).unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::Unsupported(_)));
    }

    #[test]
    fn mini_typ_lifts_int_and_range_but_rejects_placeholders() {
        let mut it = SimpleInterner::new();
        let int = mk_int_typ(&mut it);
        assert_eq!(
            mini_typ_of_canonical(&int, &it, Span::DUMMY).unwrap(),
            MiniTyp::Int
        );

        // `atom(..)` ist das geöffnete `int`-Singleton -> Int (kein Raten,
        // exakt das `open_value`-Bild).
        let atom = mk_atom_typ(&mut it);
        assert_eq!(
            mini_typ_of_canonical(&atom, &it, Span::DUMMY).unwrap(),
            MiniTyp::Int
        );

        // `TypKind::Unknown` hat kein Mini-Bild -> Unsupported.
        let unknown: Typ = Node::dummy(TypKind::Unknown);
        let err = mini_typ_of_canonical(&unknown, &it, Span::DUMMY).unwrap_err();
        assert!(matches!(err.kind, TypeErrorKind::Unsupported(_)));
    }

    #[test]
    fn mini_typ_lift_roundtrips_element_shapes() {
        let mut it = SimpleInterner::new();
        for mini in [
            MiniTyp::Int,
            MiniTyp::Bool,
            MiniTyp::Bit,
            MiniTyp::Bits(Box::new(Nexp::Const(4))),
            MiniTyp::Range(Box::new(Nexp::Const(0)), Box::new(Nexp::Const(7))),
        ] {
            let ast = ast_of_mini_typ(&mini, &mut it);
            let back = crate::conv::typ_from_ast(&ast, &it).expect("lift roundtrips");
            assert_eq!(back, mini);
        }
    }

    #[test]
    fn root_extraction_delegates_invalid_shape() {
        let mut it = SimpleInterner::new();
        let int = mk_int_typ(&mut it);
        let mut union = crate::vector_lexp::Union::over(vec![&it as &dyn sail_ast::Interner]);
        let err = vector_root_of_canonical(&int, &mut union, Span::DUMMY).unwrap_err();
        assert!(
            matches!(err.kind, TypeErrorKind::Unsupported(ref msg) if msg == "Invalid vector assignment l-expression"),
            "unexpected {err:?}"
        );
    }

    #[test]
    fn order_mapping_has_no_fallback() {
        assert_eq!(mini_order_of_ast(&AstOrder::Inc), MiniOrder::Inc);
        assert_eq!(mini_order_of_ast(&AstOrder::Dec), MiniOrder::Dec);
    }
}
