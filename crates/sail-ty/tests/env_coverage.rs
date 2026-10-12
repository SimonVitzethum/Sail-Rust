#![forbid(unsafe_code)]
//! KindEnv/TypeEnv-Endzustand (Lane 111, Env.t-Entscheid).
//!
//! E3: reiner Code, hier nicht ausgefuehrt. Friert den Endzustand ein:
//! - `KindEnv`: binden/suchen/iterieren/zaehlen/entfernen (vollstaendig).
//! - `TypeEnv`: val-specs + registers + locals sind vollstaendig
//!   (hinzufuegen/suchen/iterieren/sortierte Namen/zaehlen/entfernen,
//!   val/register-Namensraeume getrennt, `Clone` billig via `Arc`).
//! - Scattered: EINGEFROREN — kein offenes/impl/end-State im Checker.
//!   Offene/impl/end-Verkettung gehoert `sail-resolve`
//!   (`scattered_functions/_mappings/_variants/_enums`,
//!   `insert_scattered`/`lookup_scattered`); der Checker sieht nur
//!   geschlossene Schemes via `add_val`. Dieser Test haelt fest, dass
//!   `TypeEnv` dafuer bewusst keine API anbietet (val/register reicht).
//!   Details: `WORK_STATE.md` (Repo-Root).

use sail_ty::kind::KindEnv;
use sail_ty::{Kind, MiniTyp, TypeEnv, TypQuant};

// Lane-08: `Program` Records/Structs-Tabellen (OCaml `Env.records` /
// `Env.unions` / `Env.accessors`). Der Konstruktor-Kind-Eintrag lebt in
// `constructors`; diese Tabellen erhalten die Feld-Payloads (jeder
// Feldtyp wie deklariert) plus das quantifizierte Teleskop — mit
// Deklarations-Historie: Eine Sicht bei `position` sieht nie eine
// spaetere Deklaration (Quellpositions-Sichtbarkeit wie `value`).
use sail_ast::{Def, Interner as _, SimpleInterner, Span, TypKind};
use sail_parse::{ParserContext, RealParser};
use sail_ty::program::Program;
use std::sync::Arc;

fn parse_defs(sources: &[&str]) -> Vec<(Def, Arc<SimpleInterner>)> {
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

fn declare_all(sources: &[&str]) -> (Program, Vec<(Def, Arc<SimpleInterner>)>) {
    let defs = parse_defs(sources);
    let mut program = Program::new();
    for (position, (def, it)) in defs.iter().enumerate() {
        program.declare(def, it.clone(), position).unwrap();
    }
    (program, defs)
}

fn payload_head(it: &SimpleInterner, typ: &sail_ast::Typ) -> String {
    match &typ.kind {
        TypKind::Id(name) => it.resolve(name.symbol).unwrap().to_owned(),
        TypKind::App { name, .. } => it.resolve(name.symbol).unwrap().to_owned(),
        _ => panic!("expected Id/App payload, got {:?}", typ.kind),
    }
}

fn mono_int() -> TypQuant {
    TypQuant::monomorphic(MiniTyp::Int)
}

#[test]
fn kind_env_final_state() {
    let mut env = KindEnv::new();
    assert!(env.is_empty());
    env.bind("n", Kind::KInt);
    env.bind("b", Kind::KBool);
    env.bind("t", Kind::KType);
    assert_eq!(env.len(), 3);
    assert_eq!(env.lookup("n"), Some(Kind::KInt));
    assert_eq!(env.lookup("missing"), None);
    assert_eq!(env.iter().count(), 3);
    assert_eq!(env.remove("n"), Some(Kind::KInt));
    assert_eq!(env.len(), 2);
}

#[test]
fn val_specs_final_state() {
    let mut env = TypeEnv::new();
    assert_eq!(env.val_count(), 0);
    assert!(!env.contains_val("f"));
    env.add_val("f", mono_int());
    env.add_val("g", mono_int());
    assert_eq!(env.val_count(), 2);
    assert_eq!(env.val_names(), vec!["f".to_string(), "g".to_string()]);
    assert_eq!(env.iter_vals().count(), 2);
    // Re-Deklaration: last wins, keine Duplikate.
    env.add_val("f", mono_int());
    assert_eq!(env.val_count(), 2);
    assert!(env.remove_val("f").is_some());
    assert!(!env.contains_val("f"));
    assert_eq!(env.remove_val("f"), None);
}

#[test]
fn registers_final_state() {
    let mut env = TypeEnv::new();
    assert_eq!(env.register_count(), 0);
    env.add_register("xlen", mono_int());
    env.add_register("pc", mono_int());
    assert_eq!(env.register_count(), 2);
    assert_eq!(env.register_names(), vec!["pc".to_string(), "xlen".to_string()]);
    assert_eq!(env.iter_registers().count(), 2);
    assert!(env.lookup_register("xlen").is_some());
    assert!(env.lookup_register("missing").is_none());
    assert!(env.remove_register("pc").is_some());
    assert!(!env.contains_register("pc"));
}

#[test]
fn val_register_namespaces_are_independent() {
    let mut env = TypeEnv::new();
    env.add_val("r", mono_int());
    env.add_register("r", mono_int());
    // Gleicher Name in beiden Raeumen: beide sichtbar, getrennt loeschbar.
    assert!(env.lookup_val("r").is_some());
    assert!(env.lookup_register("r").is_some());
    assert!(env.remove_val("r").is_some());
    assert!(env.lookup_register("r").is_some());
    // Scattered-Freeze: offene/impl/end-Zustaende gibt es hier nicht —
    // der Checker konsumiert nur geschlossene Schemes (s. `add_val`).
    // Aufgeloeste Streu-Definitionen landen als normale vals:
    env.add_val("scattered_f_closed", mono_int());
    assert!(env.contains_val("scattered_f_closed"));
    assert_eq!(env.lookup("scattered_f_closed"), Some(MiniTyp::Int));
}

#[test]
fn locals_scope_pop_keeps_program_tables() {
    let mut env = TypeEnv::new();
    env.add_val("f", mono_int());
    env.add_register("xlen", mono_int());
    env.add_local("x", MiniTyp::Bool);
    assert_eq!(env.local_count(), 1);
    let fork = env.clone();
    assert_eq!(fork.val_count(), 1);
    assert_eq!(fork.register_count(), 1);
    let mut fork = fork;
    fork.clear_locals();
    assert_eq!(fork.local_count(), 0);
    // Programmtabellen ueberleben den Scope-Pop.
    assert!(fork.contains_val("f"));
    assert!(fork.contains_register("xlen"));
}

#[test]
fn program_struct_table_preserves_field_payloads() {
    let (program, _) = declare_all(&["struct Point = { x : int, y : bool }"]);
    let record = program.record("Point", 0).expect("struct declared");
    assert!(record.quant.is_empty());
    assert_eq!(record.fields.len(), 2);
    assert_eq!(record.fields[0].name, "x");
    assert_eq!(record.fields[1].name, "y");
    // Payloads bleiben durchgaengig erhalten (kein Kind-Only-Eintrag).
    assert_eq!(payload_head(&record.interner, &record.fields[0].typ), "int");
    assert_eq!(payload_head(&record.interner, &record.fields[1].typ), "bool");
    // Accessor-Schemata spiegeln OCaml `Env.accessors`: Fn(record) -> Feld.
    for field in ["x", "y"] {
        let scheme = program.accessor("Point", field, 0).expect("accessor");
        let TypKind::Fn { args, ret } = &scheme.typschm.typ.kind else {
            panic!("accessor is an arrow");
        };
        assert_eq!(args.len(), 1);
        assert_eq!(payload_head(&scheme.interner, ret), if field == "x" { "int" } else { "bool" });
    }
    assert_eq!(program.record_names(0), vec!["Point".to_string()]);
    assert!(program.record("Missing", 0).is_none());
    assert!(program.accessor("Point", "missing", 0).is_none());
    assert!(program.accessor("Missing", "x", 0).is_none());
}

#[test]
fn program_union_table_preserves_ctor_payloads() {
    let (program, _) = declare_all(&["union Shape = { Circle : int, Square : bool }"]);
    let union = program.variant("Shape", 0).expect("union declared");
    assert!(union.quant.is_empty());
    assert_eq!(union.ctors.len(), 2);
    assert_eq!(union.ctors[0].name, "Circle");
    assert_eq!(union.ctors[1].name, "Square");
    assert_eq!(payload_head(&union.interner, &union.ctors[0].typ), "int");
    assert_eq!(payload_head(&union.interner, &union.ctors[1].typ), "bool");
    // Konstruktor-Werteschemata bleiben daneben erhalten: Fn(Payload) -> Union.
    for ctor in ["Circle", "Square"] {
        let scheme = program.value(ctor, 0).expect("ctor value");
        assert!(matches!(scheme.typschm.typ.kind, TypKind::Fn { .. }));
    }
    assert_eq!(program.variant_names(0), vec!["Shape".to_string()]);
    assert!(program.variant("Missing", 0).is_none());
}

#[test]
fn program_record_union_history_is_position_scoped() {
    let (program, _) = declare_all(&[
        "struct A = { f : int }",
        "val v : int -> int",
        "struct B = { g : bool }",
    ]);
    // Quellpositions-Sichtbarkeit: spaetere Deklaration unsichtbar.
    assert!(program.record("A", 0).is_some());
    assert!(program.record("B", 1).is_none());
    assert!(program.record("B", 2).is_some());
    assert!(program.record("A", 2).is_some(), "fruehere bleibt sichtbar");
    assert_eq!(program.record_names(1), vec!["A".to_string()]);
    assert_eq!(program.record_names(2), vec!["A".to_string(), "B".to_string()]);
    assert!(program.variant("B", 2).is_none(), "Struct/Union-Raeume getrennt");
}

#[test]
fn program_record_rejects_duplicates() {
    // Doppelter Typkonstruktor (wie `constructors`-Tabelle).
    let defs = parse_defs(&["struct A = { f : int }", "struct A = { g : bool }"]);
    let mut program = Program::new();
    program.declare(&defs[0].0, defs[0].1.clone(), 0).unwrap();
    assert!(program.declare(&defs[1].0, defs[1].1.clone(), 1).is_err());
    // Sicht bei Position 0 sieht weiter nur die erste Deklaration.
    assert_eq!(program.record("A", 0).unwrap().fields[0].name, "f");

    // Doppeltes Feld innerhalb eines Structs.
    let defs = parse_defs(&["struct Dup = { f : int, f : bool }"]);
    let mut program = Program::new();
    assert!(program.declare(&defs[0].0, defs[0].1.clone(), 0).is_err());
    assert!(program.record("Dup", 0).is_none());
}

#[test]
fn program_quantified_struct_keeps_telescope_and_payloads() {
    let (program, _) = declare_all(&["struct Sized('n : Int) = { field : bitvector('n) }"]);
    let record = program.record("Sized", 0).expect("declared");
    assert_eq!(record.quant.len(), 1);
    assert_eq!(payload_head(&record.interner, &record.fields[0].typ), "bitvector");
    let scheme = program.accessor("Sized", "field", 0).expect("accessor");
    assert_eq!(scheme.typschm.quant.len(), 1, "Teleskop am Accessor");
    let TypKind::Fn { args, ret } = &scheme.typschm.typ.kind else {
        panic!("accessor is an arrow");
    };
    assert_eq!(args.len(), 1);
    assert_eq!(payload_head(&scheme.interner, &args[0]), "Sized");
    assert_eq!(payload_head(&scheme.interner, ret), "bitvector");
}
