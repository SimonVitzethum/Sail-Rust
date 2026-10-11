//! Active regression tests for generated grammar and core AST conversion.

#![forbid(unsafe_code)]

use sail_ast::Interner as _;
use sail_ast::{DefKind, ExpKind, PatKind, TypArgKind, TypKind, TypeDefKind};
use sail_parse::{
    FileId, RealParser, parse_def, parse_exp, parse_file, parse_file_real, parse_pat, parse_typ,
};

fn fixture(name: &str) -> String {
    let dir = env!("CARGO_MANIFEST_DIR");
    std::fs::read_to_string(format!("{dir}/tests/fixtures/{name}")).unwrap()
}

// ---------------------------------------------------------------------------
// Aktiv: Fehler tragen Spans (gilt mit und ohne generierte Tabellen).
// ---------------------------------------------------------------------------

#[test]
fn fragment_errors_carry_spans() {
    // Lexikalisch ungültig: Lexer-Fehler mit Fragment-Span, kein Panic.
    let err = parse_exp("`nope`").unwrap_err();
    assert_eq!(err.span.file, FileId(0));
    assert_eq!((err.span.start, err.span.end), (0, 6));
    assert!(err.message.contains("unexpected character"), "got: {}", err.message);

    // Leeres Fragment: Grammatik-Fehler (heute Skelett, später Syntax).
    let err = parse_typ("").unwrap_err();
    assert_eq!(err.span.file, FileId(0));
    assert_eq!((err.span.start, err.span.end), (0, 0));
    assert!(!err.message.is_empty());

    // Fehler-Display enthält Span + Meldung.
    let text = err.to_string();
    assert!(text.contains("parse error"), "got: {text}");

    // Alle Einstiege melden Fehler mit gültigem Span.
    let e_pat = parse_pat("`x`").unwrap_err();
    assert!(e_pat.span.start <= e_pat.span.end);
    let e_def = parse_def("`x`").unwrap_err();
    assert!(e_def.span.start <= e_def.span.end);
}

#[test]
fn real_parser_starts_empty_and_reports_spans() {
    let mut p = RealParser::new();
    assert!(p.interner().is_empty());
    let span = sail_parse::Span::new(FileId(0), 0, 4);
    let err = p.parse_exp("`ab`", span).unwrap_err();
    assert_eq!(err.span, span);
}

// ---------------------------------------------------------------------------
// Aktiv: Datei-Ebene ist verlustfrei (delegiert + parkt).
// ---------------------------------------------------------------------------

#[test]
fn file_real_wraps_empty_file_in_shard_markers() {
    let out = parse_file_real("empty.sail", "\n  // only a comment\n").unwrap();
    assert_eq!(out.files, vec!["empty.sail".to_owned()]);
    assert_eq!(out.ast.defs.len(), 2);
    assert!(out.ast.defs.iter().all(|d| matches!(
        d.kind,
        DefKind::Pragma { .. }
    )));
    assert!(out.deferred.is_empty());
    assert!(out.unparsed.is_empty());
}

#[test]
fn file_real_is_lossless_over_fixtures() {
    for name in ["fundefs.sail", "registers_types.sail", "pragma_shards.sail", "scattered.sail"] {
        let src = fixture(name);
        let parsed = parse_file(name, &src).unwrap();
        let out = parse_file_real(name, &src).unwrap();
        assert_eq!(out.files, vec![name.to_owned()]);
        // Nichts geht verloren: gebrückt + deferred = geparst.
        assert_eq!(
            out.ast.defs.len() + out.deferred.len(),
            parsed.defs.len(),
            "lossless violated for {name}"
        );
        // IDs sind je Datei eindeutig (Brücke + Echt-Pfad teilen einen Raum).
        let mut ids: Vec<_> = out.ast.defs.iter().map(|d| d.id).collect();
        ids.sort_by_key(|id| (id.index, id.generation));
        ids.dedup();
        assert_eq!(ids.len(), out.ast.defs.len(), "duplicate NodeIds in {name}");
    }
}

// ---------------------------------------------------------------------------
// Grammar-backed successful conversions.
// ---------------------------------------------------------------------------

#[test]
fn real_typ_bool_is_id() {
    let out = parse_typ("bool").unwrap();
    match &out.value.kind {
        TypKind::Id(n) => {
            use sail_ast::Interner as _;
            assert_eq!(out.interner.resolve(n.symbol), Some("bool"));
            assert!(!out.value.span.is_dummy());
        }
        other => panic!("expected Id, got {other:?}"),
    }
}

#[test]
fn real_exp_app_and_let() {
    let app = parse_exp("f(x, 1)").unwrap();
    assert!(matches!(app.value.kind, ExpKind::App { .. }), "{:?}", app.value.kind);
    let letexp = parse_exp("let x = 1 in x").unwrap();
    match &letexp.value.kind {
        ExpKind::Let { pat, .. } => assert!(matches!(pat.kind, PatKind::Id(_))),
        other => panic!("expected Let, got {other:?}"),
    }
}

#[test]
fn real_pat_some_is_app() {
    let out = parse_pat("Some(x)").unwrap();
    match &out.value.kind {
        PatKind::App { func, args } => {
            use sail_ast::Interner as _;
            assert_eq!(out.interner.resolve(func.symbol), Some("Some"));
            assert_eq!(args.len(), 1);
        }
        other => panic!("expected App, got {other:?}"),
    }
}

#[test]
fn real_def_val_has_typschm() {
    let out = parse_def("val f : int -> bool").unwrap();
    match &out.value.kind {
        DefKind::Val(spec) => {
            use sail_ast::Interner as _;
            assert_eq!(out.interner.resolve(spec.name.symbol), Some("f"));
            assert!(matches!(spec.typschm.typ.kind, TypKind::Fn { .. }));
        }
        other => panic!("expected Val, got {other:?}"),
    }
}

#[test]
fn file_real_lowers_let_register_constraint() {
    let src = "let x = 1\nregister R : bits(64)\nconstraint true\n";
    let out = parse_file_real("mini.sail", src).unwrap();
    assert!(out.ast.defs.iter().any(|d| matches!(
        d.kind,
        DefKind::Let { .. }
    )));
    assert!(out.ast.defs.iter().any(|d| matches!(
        d.kind,
        DefKind::Register(_)
    )));
    assert!(out.ast.defs.iter().any(|d| matches!(
        d.kind,
        DefKind::Constraint(_)
    )));
    assert!(out.deferred.is_empty(), "deferred: {:?}", out.deferred);
}

#[test]
fn file_real_lowers_bridge_bodies() {
    let src = "type t = bits(64)\ntermination_measure f(x) = x\noutcome o : int -> int\ninstantiation i with a = b\n";
    let out = parse_file_real("mini.sail", src).unwrap();
    // Echt (nicht Skelett): Typ-Rumpf aufgelöst statt `Unknown` ...
    let ty = out
        .ast
        .defs
        .iter()
        .find_map(|d| match &d.kind {
            DefKind::Type(t) => Some(t),
            _ => None,
        })
        .expect("Type fehlt");
    match &ty.kind {
        TypeDefKind::Abbrev { arg, .. } => match &arg.kind {
            TypArgKind::Typ(t) => {
                assert!(
                    matches!(t.kind, TypKind::App { .. }),
                    "Skelett-Unknown statt App: {:?}",
                    t.kind
                );
            }
            other => panic!("erwarte Typ-Arg, got {other:?}"),
        },
        other => panic!("erwarte Abbrev, got {other:?}"),
    }
    // ... Measure mit echtem Muster statt `Wild` ...
    assert!(out.ast.defs.iter().any(|d| matches!(
        &d.kind,
        DefKind::Measure { pat, .. } if matches!(pat.kind, PatKind::Id(_))
    )));
    // ... Outcome mit echtem Typschema statt `Unknown` ...
    assert!(out.ast.defs.iter().any(|d| matches!(
        &d.kind,
        DefKind::Outcome { typschm, .. } if matches!(typschm.typ.kind, TypKind::Fn { .. })
    )));
    assert!(out.ast.defs.iter().any(|d| matches!(
        d.kind,
        DefKind::Instantiation { .. }
    )));
    assert!(out.deferred.is_empty(), "deferred: {:?}", out.deferred);
}

#[test]
fn builtin_type_arguments_keep_their_sorts_and_refined_names() {
    let list = parse_typ("list(bool)").unwrap();
    assert!(matches!(&list.value.kind, TypKind::App { args, .. }
        if matches!(args[0].kind, TypArgKind::Typ(_))));
    let refined = parse_typ("bool('p & 'q)").unwrap();
    match &refined.value.kind {
        TypKind::App { name, args } => {
            assert_eq!(refined.interner.resolve(name.symbol), Some("atom_bool"));
            assert!(matches!(args[0].kind, TypArgKind::Bool(_)));
        }
        other => panic!("{other:?}"),
    }
    let refined = parse_typ("int(4)").unwrap();
    match &refined.value.kind {
        TypKind::App { name, .. } => assert_eq!(refined.interner.resolve(name.symbol), Some("atom")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn deprecated_order_parameters_are_erased_with_their_binders() {
    for source in ["vector(4, dec, bool)", "vector(4, inc, bool)", "vector(4, bool)"] {
        let out = parse_typ(source).unwrap();
        assert!(matches!(&out.value.kind, TypKind::App { args, .. }
            if args.len() == 2 && matches!(args[0].kind, TypArgKind::NExp(_))
                && matches!(args[1].kind, TypArgKind::Typ(_))), "{source}: {:?}", out.value);
    }
    let out = parse_def("val shift : forall ('n : Int) ('ord : Order). (bitvector('n, 'ord), int) -> bitvector('n, 'ord)").unwrap();
    let DefKind::Val(spec) = &out.value.kind else { panic!("{:?}", out.value); };
    assert_eq!(spec.typschm.quant.len(), 1);
    assert!(parse_typ("vector(4, bool, bool)").is_err());
    assert!(parse_typ("bitvector(4, 9)").is_err());
}

#[test]
fn vector_access_updates_and_lvalue_suffix_chains_match_ocaml_lowering() {
    let out = parse_exp("r.a[i].b[7 .. 0] = x").unwrap();
    let ExpKind::Assign { lexp, .. } = &out.value.kind else { panic!("{:?}", out.value); };
    let sail_ast::LExpKind::VectorRange { lexp, .. } = &lexp.kind else { panic!("{lexp:?}"); };
    let sail_ast::LExpKind::Field { lexp, .. } = &lexp.kind else { panic!("{lexp:?}"); };
    assert!(matches!(lexp.kind, sail_ast::LExpKind::Vector { .. }));
    for (source, function, arity) in [
        ("v[i]", "vector_access#", 2),
        ("v[7 .. 0]", "vector_subrange#", 3),
        ("v[0, 8]", "slice", 3),
        ("[v with i = x]", "vector_update#", 3),
        ("[v with 7 .. 0 = y]", "vector_update_subrange#", 4),
    ] {
        let out = parse_exp(source).unwrap();
        let ExpKind::App { func, args } = &out.value.kind else { panic!("{source}: {:?}", out.value); };
        assert_eq!(out.interner.resolve(func.symbol), Some(function));
        assert_eq!(args.len(), arity);
    }
    let out = parse_exp("[v with i = x, j = y]").unwrap();
    assert!(matches!(&out.value.kind, ExpKind::App { args, .. }
        if matches!(args[0].kind, ExpKind::App { .. })));
    assert!(matches!(parse_exp("*p = x").unwrap().value.kind,
        ExpKind::Assign { lexp, .. } if matches!(lexp.kind, sail_ast::LExpKind::Deref(_))));
    assert!(parse_exp("(*p, x) = y").is_err());
    assert!(parse_exp("v[0, 8] = x").is_err());
}

#[test]
fn descending_loops_and_legacy_bitfield_calls_keep_their_semantics() {
    for (source, order) in [
        ("foreach (i from 7 downto 0) ()", sail_ast::Order::Dec),
        ("foreach (i from 0 to 7) ()", sail_ast::Order::Inc),
    ] {
        assert!(matches!(parse_exp(source).unwrap().value.kind,
            ExpKind::For { order: actual, .. } if actual == order));
    }
    for (source, reference, arity) in [("x.f()", false, 1), ("R->f(2)", true, 2)] {
        let out = parse_exp(source).unwrap();
        let ExpKind::App { func, args } = &out.value.kind else { panic!("{:?}", out.value); };
        assert_eq!(out.interner.resolve(func.symbol), Some("_mod_f"));
        assert_eq!(args.len(), arity);
        assert_eq!(matches!(args[0].kind, ExpKind::Ref(_)), reference);
    }
}

#[test]
fn bidirectional_mappings_preserve_both_patterns_and_guards() {
    use sail_ast::{MapclKind, MPExpKind};
    let out = parse_def("mapping m = { x if p(x) <-> y if q(y) }").unwrap();
    let DefKind::Mapdef { clauses, .. } = &out.value.kind else { panic!("{:?}", out.value); };
    let MapclKind::Bidir { forwards, backwards } = &clauses[0].kind else { panic!("{:?}", clauses); };
    for (side, binding, predicate) in [(forwards, "x", "p"), (backwards, "y", "q")] {
        let MPExpKind::When { pat, guard } = &side.kind else { panic!("{side:?}"); };
        let PatKind::Id(name) = &pat.kind else { panic!("{pat:?}"); };
        assert_eq!(out.interner.resolve(name.symbol), Some(binding));
        let ExpKind::App { func, .. } = &guard.kind else { panic!("{guard:?}"); };
        assert_eq!(out.interner.resolve(func.symbol), Some(predicate));
    }
    let out = parse_def("mapping m = { x <-> y when p(x) }").unwrap();
    let DefKind::Mapdef { clauses, .. } = &out.value.kind else { panic!("{:?}", out.value); };
    assert!(matches!(&clauses[0].kind, MapclKind::Bidir { forwards, backwards }
        if matches!(forwards.kind, MPExpKind::When { .. }) && matches!(backwards.kind, MPExpKind::When { .. })));
    for source in [
        "mapping m = { x if p(x) <-> y when q(x) }",
        "mapping m = { x <-> y if p(y) when q(y) }",
    ] {
        assert!(parse_def(source).is_err(), "conflicting if/when guards: {source}");
    }
}

#[test]
fn outcome_bodies_are_retained_and_their_fixities_stay_local() {
    let mut parser = RealParser::new();
    let out = parser.parse_def("outcome read : int -> int with 'a : Type = {\ninfixl 6 %%\nval helper : (int, int) -> int\nimpl emulator(x) = x %% x\n}", sail_ast::Span::DUMMY).unwrap();
    let DefKind::Outcome { defs, .. } = &out.kind else { panic!("{out:?}"); };
    assert_eq!(defs.len(), 3);
    assert!(matches!(defs[1].kind, DefKind::Val(_)));
    assert!(matches!(&defs[2].kind, DefKind::Impl(clause)
        if matches!(&clause.arm.kind, sail_ast::PExpKind::PatExp { body, .. }
            if matches!(body.kind, ExpKind::App { .. }))));
    assert!(parser.parse_exp("x %% y", sail_ast::Span::DUMMY).is_err());
}

#[test]
fn nat_quantifiers_retain_nonnegative_constraints_and_finite_types_elaborate() {
    let out = parse_def("val f : forall ('n : Nat). int('n) -> int('n)").unwrap();
    let DefKind::Val(spec) = &out.value.kind else { panic!("{:?}", out.value); };
    assert!(matches!(&spec.typschm.quant[0], sail_ast::QuantItem::Id(kopt) if kopt.kind == sail_ast::Kind::Int));
    assert!(matches!(&spec.typschm.quant[1], sail_ast::QuantItem::Constraint(nc)
        if matches!(&nc.kind, sail_ast::ConstraintKind::Ge(_, zero)
            if matches!(&zero.kind, sail_ast::NExpKind::Constant(value) if value == "0"))));
    let out = parse_typ("{32, 64}").unwrap();
    let TypKind::Exist { binders, constraint, typ } = &out.value.kind else { panic!("{:?}", out.value); };
    assert_eq!(binders.len(), 1);
    assert!(matches!(&constraint.kind, sail_ast::ConstraintKind::Set { members, .. } if members == &["32", "64"]));
    assert!(matches!(&typ.kind, TypKind::App { name, args }
        if out.interner.resolve(name.symbol) == Some("atom") && matches!(&args[0].kind,
            TypArgKind::NExp(n) if matches!(n.kind, sail_ast::NExpKind::Var(kid) if kid == binders[0].kid))));
}

#[test]
fn negated_and_conditional_constraints_keep_boolean_arguments() {
    let mut parser = RealParser::new();
    let out = parser.parse_constraint("not('n >= 0 & 'n < 8)", sail_ast::Span::DUMMY).unwrap();
    assert!(matches!(&out.kind, sail_ast::ConstraintKind::App { func, args }
        if parser.interner().resolve(func.symbol) == Some("not")
            && matches!(&args[0].kind, TypArgKind::Bool(nc) if matches!(nc.kind, sail_ast::ConstraintKind::And(..)))));
    let out = parser.parse_constraint("if 'cheri then 'nt >= 0 else 'nt == 0", sail_ast::Span::DUMMY).unwrap();
    assert!(matches!(&out.kind, sail_ast::ConstraintKind::Or(yes, no)
        if matches!(yes.kind, sail_ast::ConstraintKind::And(..)) && matches!(no.kind, sail_ast::ConstraintKind::And(..))));
    assert!(parser.parse_constraint("not(1, 2)", sail_ast::Span::DUMMY).is_err());
}

#[test]
fn omitted_binder_kinds_follow_their_uses_and_explicit_kinds_are_checked() {
    for (source, expected) in [
        ("val identity : forall 'a. 'a -> 'a", sail_ast::Kind::Type),
        ("val width : forall 'n, 'n >= 0. bitvector('n) -> int('n)", sail_ast::Kind::Int),
        ("val predicate : forall 'p. unit -> bool(not('p))", sail_ast::Kind::Bool),
    ] {
        let out = parse_def(source).unwrap();
        let DefKind::Val(spec) = &out.value.kind else { panic!("{:?}", out.value); };
        assert!(matches!(&spec.typschm.quant[0], sail_ast::QuantItem::Id(kopt)
            if kopt.kind == expected), "{source}: {:?}", spec.typschm.quant);
    }
    for source in [
        "val bad : forall ('n : Type). bitvector('n) -> unit",
        "val bad : forall 'a. ('a, bitvector('a)) -> unit",
        "val unused : forall 'a. unit -> unit",
    ] {
        assert!(parse_def(source).is_err(), "{source}");
    }
}

#[test]
fn constructor_arguments_use_bound_kinds_and_inference_respects_shadowing() {
    let out = parse_def("val f : forall 'a. T('a) -> 'a").unwrap();
    let DefKind::Val(spec) = &out.value.kind else { panic!("{:?}", out.value); };
    assert!(matches!(&spec.typschm.typ.kind, TypKind::Fn { args, .. }
        if matches!(&args[0].kind, TypKind::App { args, .. }
            if matches!(args[0].kind, TypArgKind::Typ(_)))));
    let out = parse_def("val f : forall 'n. (bitvector('n), {'n, 'n >= 0. int('n)}) -> int('n)").unwrap();
    assert!(matches!(&out.value.kind, DefKind::Val(spec)
        if matches!(&spec.typschm.quant[0], sail_ast::QuantItem::Id(kopt) if kopt.kind == sail_ast::Kind::Int)));
    parse_def("val f : forall 'a. ('a, {('a : Int), 'a >= 0. int('a)}) -> 'a").unwrap();
    let inferred = parse_def("type Numeric = 2048").unwrap();
    assert!(matches!(&inferred.value.kind, DefKind::Type(ty)
        if matches!(&ty.kind, TypeDefKind::Abbrev { arg, .. } if matches!(arg.kind, TypArgKind::NExp(_)))));
    parse_def("val legacy : int -> unit effect {escape}").unwrap();
}

#[test]
fn setter_assignments_append_rhs_once_and_keep_ocaml_argument_shapes() {
    for (source, name, arity) in [
        ("_R(n) = compute()", "_R", 2),
        ("set() = compute()", "set", 1),
        ("set((a, b)) = compute()", "set", 3),
        ("set(a, (b, c)) = compute()", "set", 3),
        ("x.f() = compute()", "_mod_f", 2),
    ] {
        let out = parse_exp(source).unwrap();
        let ExpKind::App { func, args } = &out.value.kind else { panic!("{:?}", out.value); };
        assert_eq!(out.interner.resolve(func.symbol), Some(name));
        assert_eq!(args.len(), arity, "{source}");
        assert!(matches!(&args.last().unwrap().kind, ExpKind::App { func, .. }
            if out.interner.resolve(func.symbol) == Some("compute")));
        assert_eq!(out.value.annotation.as_ref().unwrap().attributes[0].name, "setter");
    }
    for source in ["var set() = 1 in ()", "(set(1), x) = (2, 3)", "set(1)[0] = 1"] {
        assert!(parse_exp(source).is_err(), "{source}");
    }
}

#[test]
fn source_annotations_survive_conversion_and_serialization() {
    let out = parse_def("/// kept documentation\nprivate $[never_fold, payload { enabled = true, values = [1, \"x\"] }] val f : unit -> unit").unwrap();
    let annotation = out.value.annotation.as_ref().unwrap();
    assert!(annotation.private);
    assert!(annotation.documentation.as_ref().unwrap().contains("kept documentation"));
    assert_eq!(annotation.attributes[0].name, "never_fold");
    assert!(matches!(&annotation.attributes[1].data, Some(sail_ast::AttributeData::Object(fields))
        if matches!(fields[0].1, sail_ast::AttributeData::Bool(true))));
    let json = serde_json::to_vec(&out.value).unwrap();
    assert_eq!(serde_json::from_slice::<sail_ast::Def>(&json).unwrap(), out.value);
    assert!(parse_exp("$[never_fold] f()").unwrap().value.annotation.is_some());
    assert!(parse_pat("$[payload 1] x").unwrap().value.annotation.is_some());
    assert!(parse_exp("x").unwrap().value.annotation.is_none());
}

#[test]
fn declared_constructors_infer_kinds_without_retaining_their_bodies() {
    let mut parser = RealParser::new();
    let span = sail_ast::Span::DUMMY;
    parser.parse_def("type Wrap('a : Type) = list('a)", span).unwrap();
    parser.parse_def("type Predicate('p : Bool) -> Bool = not('p)", span).unwrap();
    for (source, expected) in [
        ("val f : forall 'a. Wrap('a) -> unit", sail_ast::Kind::Type),
        ("val f : forall 'p. unit -> bool(Predicate('p))", sail_ast::Kind::Bool),
    ] {
        let out = parser.parse_def(source, span).unwrap();
        assert!(matches!(&out.kind, DefKind::Val(spec)
            if matches!(&spec.typschm.quant[0], sail_ast::QuantItem::Id(kopt) if kopt.kind == expected)), "{out:?}");
    }
    assert!(parser.parse_def("val f : forall ('n : Int). Wrap('n) -> unit", span).is_err());
    assert!(parser.parse_def("val f : Wrap(int, bool) -> unit", span).is_err());
    parser.parse_def("type COUNT : Int = 4", span).unwrap();
    parser.parse_def("type ENABLED : Bool = true", span).unwrap();
    parser.parse_def("struct S = { w : Wrap(int), b : bool(ENABLED), n : bits(COUNT) }", span).unwrap();
}

#[test]
fn declaration_context_snapshots_and_failures_do_not_leak_signatures() {
    let span = sail_ast::Span::DUMMY;
    let mut parser = RealParser::new();
    let previous = parser.context();
    parser.parse_def("type Later('a : Type) = list('a)", span).unwrap();
    let source = "val f : forall 'a. Later('a) -> unit";
    assert!(RealParser::with_context(previous).parse_def(source, span).is_err());
    RealParser::with_context(parser.context()).parse_def(source, span).unwrap();
    assert!(parser.parse_def("struct Bad('a : Type) = { a : 'a, bad : list(1) }", span).is_err());
    assert!(parser.parse_def("val f : forall 'a. Bad('a) -> unit", span).is_err());
    parser.parse_def("outcome o : unit -> unit with 'ctx : Type = { type Local('a : Type) = list('a) }", span).unwrap();
    assert!(parser.parse_def("val f : forall 'a. Local('a) -> unit", span).is_err());
}

#[test]
fn arm_bitfield_concat_keeps_range_order_through_parentheses() {
    let out = parse_def("bitfield PMEVTYPER_EL0_Type : bits(64) = { evtCount : (15..10 @ 9..0) }").unwrap();
    let DefKind::Type(ty) = &out.value.kind else { panic!("{:?}", out.value); };
    let TypeDefKind::Bitfield { fields, .. } = &ty.kind else { panic!("{ty:?}"); };
    let sail_ast::IndexRangeKind::Concat(left, right) = &fields[0].1.kind else { panic!("{fields:?}"); };
    let bounds = |r: &sail_ast::IndexRange| {
        let sail_ast::IndexRangeKind::Range { lo, hi } = &r.kind else { panic!("{r:?}"); };
        let (sail_ast::NExpKind::Constant(lo), sail_ast::NExpKind::Constant(hi)) = (&lo.kind, &hi.kind) else { panic!("{r:?}"); };
        (lo.clone(), hi.clone())
    };
    assert_eq!(bounds(left), ("15".into(), "10".into()));
    assert_eq!(bounds(right), ("9".into(), "0".into()));
}

#[test]
fn function_domain_tuple_becomes_multiple_arguments() {
    let out = parse_def("val f : (int, bool) -> int").unwrap();
    assert!(matches!(&out.value.kind, DefKind::Val(spec)
        if matches!(&spec.typschm.typ.kind, TypKind::Fn { args, .. } if args.len() == 2)));
}

#[test]
fn infix_folding_is_connected_to_expression_and_numeric_conversion() {
    let out = parse_exp("a + b * c").unwrap();
    let ExpKind::App { func, args } = &out.value.kind else { panic!("{:?}", out.value); };
    assert_eq!(out.interner.resolve(func.symbol), Some("+"));
    let ExpKind::App { func, .. } = &args[1].kind else { panic!("{:?}", args[1]); };
    assert_eq!(out.interner.resolve(func.symbol), Some("*"));
    let width = parse_typ("bits('n + 4 * 2)").unwrap();
    assert!(matches!(&width.value.kind, TypKind::App { args, .. }
        if matches!(&args[0].kind, TypArgKind::NExp(n) if matches!(n.kind, sail_ast::NExpKind::Sum(..)))));
    parse_def("constraint 'a < 'b <= 'c & 'd >= 0").unwrap();
}

#[test]
fn custom_fixities_survive_between_fragment_calls() {
    let mut parser = RealParser::new();
    let span = sail_parse::Span::DUMMY;
    assert!(parser.parse_exp("a %% b", span).is_err());
    parser.parse_def("infixr 5 %%", span).unwrap();
    let exp = parser.parse_exp("a %% b %% c", span).unwrap();
    assert!(matches!(&exp.kind, ExpKind::App { args, .. } if matches!(args[1].kind, ExpKind::App { .. })));
}

#[test]
fn file_real_lowers_function_bodies_without_placeholder_fallback() {
    let out = parse_file_real("f.sail", "val f : (int, int) -> int\nfunction f(a, b) = a + b * 2\n").unwrap();
    assert!(out.unparsed.is_empty(), "{:?}", out.unparsed);
    assert!(matches!(&out.ast.defs[2].kind, DefKind::Fundef { funcls, .. }
        if matches!(&funcls[0].arm.kind, sail_ast::PExpKind::PatExp { body, .. }
            if matches!(body.kind, ExpKind::App { .. }))));
}

#[test]
fn numeric_and_boolean_type_abbreviations_keep_their_result_kinds() {
    let numeric = parse_def("type MAX_VL : Int = 2048").unwrap();
    assert!(matches!(&numeric.value.kind, DefKind::Type(ty)
        if matches!(&ty.kind, TypeDefKind::Abbrev { arg, .. } if matches!(arg.kind, TypArgKind::NExp(_)))));
    let boolean = parse_def("type is_VL('vl : Int) -> Bool = ('vl in {128, 256, 512, 1024, 2048})").unwrap();
    assert!(matches!(&boolean.value.kind, DefKind::Type(ty)
        if matches!(&ty.kind, TypeDefKind::Abbrev { arg, .. } if matches!(arg.kind, TypArgKind::Bool(_)))));
}
