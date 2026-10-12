//! Lane-19 (audit O-5): mapping / scattered / outcome declaration tables
//! in [`Program`](sail_ty::program::Program) with source-position
//! visibility, plus the body-check handoff baseline.
//!
//! Headers are retained at their declaration position; clause and
//! measure bodies stay in the driver store, so `check_definition` must
//! keep rejecting them with `Unsupported` until Lane-16 inserts the
//! `exp_arms` plans (pinned at the bottom of this file).

use sail_ast::*;
use sail_ty::error::TypeErrorKind;
use sail_ty::exp_arms;
use sail_ty::program::{Program, ScatteredNamespace};
use std::sync::Arc;

fn interner() -> SimpleInterner {
    SimpleInterner::new()
}

fn name(it: &mut SimpleInterner, text: &str) -> Name {
    Name::dummy(it.intern(text))
}

fn int_typ(it: &mut SimpleInterner) -> Typ {
    Node::dummy(TypKind::Id(name(it, "int")))
}

fn fn_scheme(arg: Typ, ret: Typ) -> TypSchm {
    TypSchm {
        quant: Vec::new(),
        typ: Node::dummy(TypKind::Fn {
            args: vec![arg],
            ret: Box::new(ret),
        }),
    }
}

fn val_def(it: &mut SimpleInterner, fname: &str) -> Def {
    let arg = int_typ(it);
    let ret = int_typ(it);
    Node::dummy(DefKind::Val(ValSpec {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        name: name(it, fname),
        typschm: fn_scheme(arg, ret),
        extern_opt: None,
    }))
}

fn bidir_def(it: &mut SimpleInterner, mname: &str, with_annot: bool) -> Def {
    let annot = if with_annot {
        let lhs = int_typ(it);
        let rhs = int_typ(it);
        TAnnotOpt::Some {
            quant: Vec::new(),
            typ: Node::dummy(TypKind::Bidir {
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            }),
        }
    } else {
        TAnnotOpt::None
    };
    Node::dummy(DefKind::Mapdef {
        name: name(it, mname),
        annot,
        clauses: Vec::new(),
    })
}

fn declare(program: &mut Program, def: &Def, it: &Arc<SimpleInterner>, position: usize) {
    program.declare(def, it.clone(), position).unwrap();
}

#[test]
fn mapping_header_visible_at_source_position() {
    let mut it = interner();
    let def = bidir_def(&mut it, "decode", true);
    let it = Arc::new(it);
    let mut program = Program::new();
    assert!(!program.is_mapping("decode", 0));
    declare(&mut program, &def, &it, 0);
    assert!(program.is_mapping("decode", 0));
    let mapping = program.mapping("decode", 0).unwrap();
    assert!(matches!(mapping.typ.kind, TypKind::Bidir { .. }));
    assert_eq!(
        program.mapping_expansion("decode", 0),
        Some(("decode_forwards".to_owned(), "decode_backwards".to_owned()))
    );
    assert_eq!(program.mapping_names(0), vec!["decode".to_owned()]);
    // A worker at an earlier position never sees the later declaration:
    // a second mapping declared at position 1 is invisible at 0.
    assert_eq!(program.mapping_names(0).len(), 1);
}

#[test]
fn mapping_expansion_names_match_planner_convention() {
    assert_eq!(
        exp_arms::mapping_expansion_names("decode"),
        ("decode_forwards".to_owned(), "decode_backwards".to_owned())
    );
}

#[test]
fn mapping_rejects_non_bidir_and_missing_annot() {
    let mut it = interner();
    let mut program = Program::new();
    // Non-bidirectional annotation.
    let non_bidir = Node::dummy(DefKind::Mapdef {
        name: name(&mut it, "bad"),
        annot: TAnnotOpt::Some {
            quant: Vec::new(),
            typ: int_typ(&mut it),
        },
        clauses: Vec::new(),
    });
    let it = Arc::new(it);
    let err = program.declare(&non_bidir, it.clone(), 0).unwrap_err();
    assert!(matches!(err.kind, TypeErrorKind::QuantError(_)));
    assert!(program.mapping("bad", 0).is_none());
    // Missing annotation: synthesis needs pattern elaboration (Lane-09).
    let mut it2 = interner();
    let no_annot = bidir_def(&mut it2, "synth", false);
    let it2 = Arc::new(it2);
    let err = program.declare(&no_annot, it2, 1).unwrap_err();
    assert!(matches!(err.kind, TypeErrorKind::Unsupported(_)));
}

#[test]
fn scattered_open_clause_end_lifecycle() {
    let mut it = interner();
    let open = Node::dummy(DefKind::Scattered(ScatteredDef::Function {
        name: name(&mut it, "f"),
        annot: TAnnotOpt::None,
    }));
    let funcl = Funcl {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        name: name(&mut it, "f"),
        arm: PExp {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            kind: PExpKind::PatExp {
                pat: Node::dummy(PatKind::Wild),
                body: Node::dummy(ExpKind::Undef),
            },
        },
    };
    let clause = Node::dummy(DefKind::Scattered(ScatteredDef::Funcl(funcl)));
    let end = Node::dummy(DefKind::Scattered(ScatteredDef::End(name(&mut it, "f"))));
    let it = Arc::new(it);
    let mut program = Program::new();
    declare(&mut program, &open, &it, 0);
    let decl = program
        .scattered(ScatteredNamespace::Function, "f", 0)
        .unwrap();
    assert!(!decl.closed);
    assert_eq!(
        program.scattered_clause_count(ScatteredNamespace::Function, "f", 0),
        0
    );
    declare(&mut program, &clause, &it, 1);
    declare(&mut program, &clause, &it, 2);
    assert_eq!(
        program.scattered_clause_count(ScatteredNamespace::Function, "f", 2),
        2
    );
    // Earlier workers see the earlier count.
    assert_eq!(
        program.scattered_clause_count(ScatteredNamespace::Function, "f", 1),
        1
    );
    declare(&mut program, &end, &it, 3);
    assert!(
        program
            .scattered(ScatteredNamespace::Function, "f", 3)
            .unwrap()
            .closed
    );
    assert!(
        !program
            .scattered(ScatteredNamespace::Function, "f", 2)
            .unwrap()
            .closed
    );
}

#[test]
fn scattered_clause_without_open_block_fails() {
    let mut it = interner();
    let funcl = Funcl {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        name: name(&mut it, "ghost"),
        arm: PExp {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            kind: PExpKind::PatExp {
                pat: Node::dummy(PatKind::Wild),
                body: Node::dummy(ExpKind::Undef),
            },
        },
    };
    let clause = Node::dummy(DefKind::Scattered(ScatteredDef::Funcl(funcl)));
    let end = Node::dummy(DefKind::Scattered(ScatteredDef::End(name(
        &mut it, "ghost",
    ))));
    let it = Arc::new(it);
    let mut program = Program::new();
    assert!(program.declare(&clause, it.clone(), 0).is_err());
    assert!(program.declare(&end, it, 0).is_err());
}

#[test]
fn scattered_namespaces_are_independent() {
    let mut it = interner();
    let fun_open = Node::dummy(DefKind::Scattered(ScatteredDef::Function {
        name: name(&mut it, "f"),
        annot: TAnnotOpt::None,
    }));
    let map_open = Node::dummy(DefKind::Scattered(ScatteredDef::Mapping {
        name: name(&mut it, "f"),
        annot: TAnnotOpt::None,
    }));
    let it = Arc::new(it);
    let mut program = Program::new();
    declare(&mut program, &fun_open, &it, 0);
    declare(&mut program, &map_open, &it, 1);
    assert!(program
        .scattered(ScatteredNamespace::Function, "f", 1)
        .is_some());
    assert!(program
        .scattered(ScatteredNamespace::Mapping, "f", 1)
        .is_some());
    assert!(program
        .scattered(ScatteredNamespace::Enum, "f", 1)
        .is_none());
}

#[test]
fn outcome_header_and_local_vals_do_not_leak_globally() {
    let mut it = interner();
    let inner_arg = int_typ(&mut it);
    let inner_ret = int_typ(&mut it);
    let inner_val = ValSpec {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        name: name(&mut it, "helper"),
        typschm: fn_scheme(inner_arg, inner_ret),
        extern_opt: None,
    };
    let outcome = Node::dummy(DefKind::Outcome {
        name: name(&mut it, "O"),
        typschm: TypSchm {
            quant: Vec::new(),
            typ: int_typ(&mut it),
        },
        quant: Vec::new(),
        defs: vec![Node::dummy(DefKind::Val(inner_val))],
    });
    let it = Arc::new(it);
    let mut program = Program::new();
    declare(&mut program, &outcome, &it, 0);
    assert!(program.outcome("O", 0).is_some());
    assert_eq!(program.outcome_names(0), vec!["O".to_owned()]);
    assert!(program.outcome_val("O", "helper", 0).is_some());
    // Outcome-local vals never leak into the global table.
    assert!(program.value("helper", 0).is_none());
    // Duplicate outcome rejected.
    let mut it2 = interner();
    let dup = Node::dummy(DefKind::Outcome {
        name: name(&mut it2, "O"),
        typschm: TypSchm {
            quant: Vec::new(),
            typ: int_typ(&mut it2),
        },
        quant: Vec::new(),
        defs: Vec::new(),
    });
    let it2 = Arc::new(it2);
    assert!(program.declare(&dup, it2, 1).is_err());
}

#[test]
fn instantiation_requires_known_outcome_and_is_recorded() {
    let mut it = interner();
    let inst = Node::dummy(DefKind::Instantiation {
        name: name(&mut it, "Missing"),
        substs: Vec::new(),
    });
    let it = Arc::new(it);
    let mut program = Program::new();
    assert!(program.declare(&inst, it.clone(), 0).is_err());
    assert!(!program.is_instantiated("Missing", 0));
}

#[test]
fn measure_requires_known_function_and_is_recorded() {
    let mut it = interner();
    let val = val_def(&mut it, "fact");
    let measure = Node::dummy(DefKind::Measure {
        name: name(&mut it, "fact"),
        pat: Node::dummy(PatKind::Wild),
        exp: Node::dummy(ExpKind::Undef),
    });
    let unknown_measure = Node::dummy(DefKind::Measure {
        name: name(&mut it, "ghost"),
        pat: Node::dummy(PatKind::Wild),
        exp: Node::dummy(ExpKind::Undef),
    });
    let it = Arc::new(it);
    let mut program = Program::new();
    declare(&mut program, &val, &it, 0);
    assert!(program.declare(&unknown_measure, it.clone(), 1).is_err());
    declare(&mut program, &measure, &it, 1);
    assert!(program.measure("fact", 1).is_some());
    assert!(program.measure("fact", 0).is_none());
}

#[test]
fn body_checking_still_rejects_deferred_bodies() {
    // Handoff baseline for Lane-16: headers declare, bodies stay
    // `Unsupported` until the `exp_arms` plans are inserted into
    // `canonical_body.rs` (which this lane must not edit).
    let mut it = interner();
    let def = bidir_def(&mut it, "decode", true);
    let it = Arc::new(it);
    let mut program = Program::new();
    declare(&mut program, &def, &it, 0);
    let err = program.check_definition(&def, it, 0).unwrap_err();
    assert!(matches!(err.kind, TypeErrorKind::Unsupported(_)));
}
