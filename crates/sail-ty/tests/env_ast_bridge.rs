#![forbid(unsafe_code)]
//! Canonical env views onto `sail-ast` (Lane 72, Lane-61-Reste #23/#24).
//!
//! E3: reiner Code, hier nicht ausgefuehrt. Prueft den Lane-72-Entscheid
//! "Envs bleiben lokal, Sichten aufs Kanonische":
//! - [`KindEnv`] als sortiertes `KOpt`-Teleskop (`kenv_to_kopts`) und zurueck
//!   (`kenv_from_kopts`); `KNat` erlischt zu `Int` und wird NIE zurueckgeraten.
//! - [`TypeEnv`]-Schemes (`val`/`register`) als `sail_ast::TypSchm`
//!   (`val_scheme_ast` / `register_scheme_ast`).

use sail_ast::{Interner as _, SimpleInterner};
use sail_ty::kind::KindEnv;
use sail_ty::{Kind, MiniTyp, NConstraint, Nexp, TypeEnv, TypQuant, kenv_from_kopts, kenv_to_kopts};

// Lane-08: kanonische `Program`-Sichten auf Records/Structs (OCaml
// `Env.get_record` / `Env.get_variant` / `Env.get_accessor`). Payloads
// sind bereits kanonische Knoten — die Sichten borgen statt zu senken,
// positionsbewusst wie die Tabellen selbst.
use sail_ast::{Span, TypKind};
use sail_parse::{ParserContext, RealParser};
use sail_ty::program::Program;
use std::sync::Arc;

fn parse_defs(sources: &[&str]) -> Vec<(sail_ast::Def, Arc<SimpleInterner>)> {
    let mut context = ParserContext::default();
    sources
        .iter()
        .map(|source| {
            let mut parser = RealParser::with_context(context.clone());
            let def = parser.parse_def(source, Span::DUMMY).unwrap();
            context = parser.context();
            (def, Arc::new(parser.into_interner()))
        })
        .collect()
}

fn declare_all(sources: &[&str]) -> Program {
    let defs = parse_defs(sources);
    let mut program = Program::new();
    for (position, (def, it)) in defs.iter().enumerate() {
        program.declare(def, it.clone(), position).unwrap();
    }
    program
}

fn sample_kind_env() -> KindEnv {
    let mut env = KindEnv::new();
    env.bind("n", Kind::KInt);
    env.bind("m", Kind::KNat);
    env.bind("b", Kind::KBool);
    env.bind("t", Kind::KType);
    env
}

fn quantified_bits() -> TypQuant {
    TypQuant {
        quants: vec![("n".to_string(), Kind::KInt)],
        constraint: NConstraint::Ge(
            Box::new(Nexp::Var("n".into())),
            Box::new(Nexp::Const(0)),
        ),
        typ: MiniTyp::Bits(Box::new(Nexp::Var("n".into()))),
    }
}

#[test]
fn kenv_telescope_covers_all_bindings_sorted() {
    let mut it = SimpleInterner::new();
    let kopts = kenv_to_kopts(&sample_kind_env(), &mut it);
    let names: Vec<String> = kopts
        .iter()
        .map(|k| it.resolve(k.kid).expect("binder resolves").to_owned())
        .collect();
    assert_eq!(names, vec!["b", "m", "n", "t"]);
}

#[test]
fn kenv_roundtrip_keeps_all_but_knat_refinement() {
    let mut it = SimpleInterner::new();
    let env = sample_kind_env();
    let back = kenv_from_kopts(&kenv_to_kopts(&env, &mut it), &it).expect("roundtrips");
    assert_eq!(back.lookup("n"), Some(Kind::KInt));
    assert_eq!(back.lookup("b"), Some(Kind::KBool));
    assert_eq!(back.lookup("t"), Some(Kind::KType));
    // Dokumentierte Erasure: `KNat -> Int`, kein stilles Zurueckraten.
    assert_eq!(back.lookup("m"), Some(Kind::KInt));
}

#[test]
fn type_env_schemes_lower_onto_typschm() {
    let mut it = SimpleInterner::new();
    let mut env = TypeEnv::new();
    env.add_val("mem_read", quantified_bits());
    env.add_register("xlen", TypQuant::monomorphic(MiniTyp::Int));

    let val = env.val_scheme_ast("mem_read", &mut it).expect("known val");
    // Ein Binder plus eine Constraint-Position (vgl. `typquant_to_ast`).
    assert_eq!(val.quant.len(), 2);

    let reg = env
        .register_scheme_ast("xlen", &mut it)
        .expect("known register");
    assert_eq!(reg.quant.len(), 1, "monomorphic: nur die Constraint-Position");

    assert_eq!(env.val_scheme_ast("xlen", &mut it), None, "val/register getrennt");
    assert_eq!(env.register_scheme_ast("mem_read", &mut it), None);
    assert_eq!(env.val_scheme_ast("missing", &mut it), None);
}

#[test]
fn program_record_fields_view_borrows_declared_payloads() {
    let program = declare_all(&["struct Point = { x : int, y : bool }"]);
    let fields = program.record_fields_ast("Point", 0).expect("known struct");
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].0, "x");
    assert_eq!(fields[1].0, "y");
    // Payloads sind kanonische Knoten mit Quellspanne aus der Deklaration.
    for (name, typ, span) in &fields {
        let _ = span;
        assert!(
            matches!(typ.kind, TypKind::Id(_) | TypKind::App { .. }),
            "{name}: kein Kind-Only-Platzhalter"
        );
    }
    // Sicht stimmt mit der Tabelle ueberein (gleiche Payloads, gleiche Spannen).
    let record = program.record("Point", 0).expect("table");
    for (view, kept) in fields.iter().zip(record.fields.iter()) {
        assert_eq!(view.0, kept.name);
        assert_eq!(view.2, kept.span);
    }
    assert!(program.record_fields_ast("Missing", 0).is_none());
}

#[test]
fn program_variant_ctors_view_borrows_ctor_payloads() {
    let program = declare_all(&["union Shape = { Circle : int, Square : bool }"]);
    let ctors = program.variant_ctors_ast("Shape", 0).expect("known union");
    assert_eq!(ctors.len(), 2);
    assert_eq!(ctors[0].0, "Circle");
    assert_eq!(ctors[1].0, "Square");
    let union = program.variant("Shape", 0).expect("table");
    for (view, kept) in ctors.iter().zip(union.ctors.iter()) {
        assert_eq!(view.0, kept.name);
        assert_eq!(view.2, kept.span);
    }
    // Konstruktor-Wertsichten bleiben Fn-Pfeile auf dieselben Payloads.
    for (name, payload, _) in &ctors {
        let scheme = program.value(name, 0).expect("ctor value");
        let TypKind::Fn { args, .. } = &scheme.typschm.typ.kind else {
            panic!("ctor value is an arrow");
        };
        assert_eq!(args.len(), 1);
        assert!(matches!(args[0].kind, TypKind::Id(_) | TypKind::App { .. }), "{name}");
        let _ = payload;
    }
    assert!(program.variant_ctors_ast("Missing", 0).is_none());
}

#[test]
fn program_accessor_view_is_quantified_arrow_over_telescope() {
    let program = declare_all(&["struct Sized('n : Int) = { field : bitvector('n) }"]);
    let scheme = program
        .accessor_scheme_ast("Sized", "field", 0)
        .expect("accessor");
    assert_eq!(scheme.quant.len(), 1, "Teleskop erhalten");
    let TypKind::Fn { args, ret } = &scheme.typ.kind else {
        panic!("accessor is an arrow");
    };
    assert_eq!(args.len(), 1, "ein Record-Argument");
    assert!(program.accessor_scheme_ast("Sized", "missing", 0).is_none());
    assert!(program.accessor_scheme_ast("Missing", "field", 0).is_none());
}

#[test]
fn program_record_union_views_are_position_scoped() {
    let program = declare_all(&["struct A = { f : int }", "union U = { C : int }"]);
    assert!(program.record_fields_ast("A", 0).is_some());
    assert!(program.variant_ctors_ast("U", 0).is_none(), "erst bei 1 deklariert");
    assert!(program.variant_ctors_ast("U", 1).is_some());
    assert!(program.accessor_scheme_ast("A", "f", 0).is_some());
    assert!(program.accessor_scheme_ast("A", "f", 1).is_some(), "frueher sichtbar");
}
