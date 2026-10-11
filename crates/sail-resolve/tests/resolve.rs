//! Integration tests: collision case + happy path + distincto errors.
//!
//! Fixtures build canonical `sail-ast` nodes directly (Lane 61, #9–#16)
//! with a `SimpleInterner` for names; all spans are `DUMMY`.

use sail_ast::{
    Ast, ConstraintKind, DecSpec, Def, DefKind, ExpKind, Fixity, Funcl, FunclSet, Interner, Loop,
    Mapcl, Name, Node, NodeId, Order, PExp, PExpKind, PatKind, RecOpt, ScatteredDef,
    SimpleInterner, Span, TAnnotOpt, Typ, TypKind, TypSchm, TypeDefKind, TypeUnion, ValSpec,
};
use sail_resolve::ResolveErrorKind;
use sail_resolve::resolve::resolve;

#[test]
fn deep_pattern_names_use_an_explicit_work_stack() {
    std::thread::Builder::new().stack_size(64 * 1024).spawn(|| {
        let mut interner = SimpleInterner::new();
        let symbol = interner.intern("leaf");
        let mut pat = Node::dummy(PatKind::Id(Name::dummy(symbol)));
        for _ in 0..10_000 {
            pat = Node::dummy(PatKind::Not(Box::new(pat)));
        }
        assert_eq!(sail_resolve::resolve::pat_names(&pat, &interner),
            vec![("leaf".to_owned(), Span::DUMMY)]);
        // Check the traversal separately from the AST's derived destructor.
        while let PatKind::Not(inner) = pat.kind { pat = *inner; }
    }).unwrap().join().unwrap();
}

fn nm(it: &mut SimpleInterner, text: &str) -> Name {
    Name::dummy(it.intern(text))
}

fn unknown_typ() -> Typ {
    Node::dummy(TypKind::Unknown)
}

fn struct_def(it: &mut SimpleInterner, name: &str) -> Def {
    Node::dummy(DefKind::Type(Node::dummy(TypeDefKind::Record {
        name: nm(it, name),
        quant: Vec::new(),
        fields: Vec::new(),
        wf: false,
    })))
}

fn enum_def(it: &mut SimpleInterner, owner: &str, members: &[&str]) -> Def {
    Node::dummy(DefKind::Type(Node::dummy(TypeDefKind::Enum {
        name: nm(it, owner),
        members: members.iter().map(|c| nm(it, c)).collect(),
        wf: false,
    })))
}

fn abbrev_def(it: &mut SimpleInterner, name: &str) -> Def {
    Node::dummy(DefKind::Type(Node::dummy(TypeDefKind::Abbrev {
        name: nm(it, name),
        quant: Vec::new(),
        arg: Node::dummy(sail_ast::TypArgKind::Typ(unknown_typ())),
    })))
}

fn union_def(it: &mut SimpleInterner, owner: &str, ctors: &[&str]) -> Def {
    Node::dummy(DefKind::Type(Node::dummy(TypeDefKind::Variant {
        name: nm(it, owner),
        quant: Vec::new(),
        ctors: ctors
            .iter()
            .map(|c| TypeUnion {
                id: NodeId::DUMMY,
                span: Span::DUMMY,
                typ: unknown_typ(),
                ctor: nm(it, c),
            })
            .collect(),
        wf: false,
    })))
}

fn valspec_def(it: &mut SimpleInterner, name: &str) -> Def {
    Node::dummy(DefKind::Val(ValSpec {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        name: nm(it, name),
        typschm: TypSchm { quant: Vec::new(), typ: unknown_typ() },
        extern_opt: None,
    }))
}

fn register_def(it: &mut SimpleInterner, name: &str) -> Def {
    Node::dummy(DefKind::Register(DecSpec {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        typ: unknown_typ(),
        name: nm(it, name),
        init: None,
    }))
}

fn mapping_def(it: &mut SimpleInterner, name: &str) -> Def {
    Node::dummy(DefKind::Mapdef {
        name: nm(it, name),
        annot: TAnnotOpt::None,
        clauses: Vec::<Mapcl>::new(),
    })
}

#[test]
fn happy_path_no_errors_no_renames() {
    let mut it = SimpleInterner::new();
    let ast = Ast::new(vec![
        struct_def(&mut it, "U"),
        union_def(&mut it, "V", &["A", "B"]),
        enum_def(&mut it, "E", &["X", "Y"]),
        abbrev_def(&mut it, "N"),
        valspec_def(&mut it, "f"),
        register_def(&mut it, "sp"),
        mapping_def(&mut it, "decode"),
    ]);
    let r = resolve(&ast, &it);
    assert!(r.errors.is_empty(), " {:?}", r.errors);
    assert!(r.warnings.is_empty(), " {:?}", r.warnings);
    assert!(r.ctor_renames.is_empty(), " {:?}", r.ctor_renames);
    assert!(r.env.lookup_type("U").is_some());
    assert!(r.env.lookup_ctor("B").is_some());
    assert!(r.env.lookup_valspec("f").is_some());
}

#[test]
fn streamed_definitions_with_independent_interners_match_whole_program_resolution() {
    let mut shared = SimpleInterner::new();
    let ast = Ast::new(vec![
        struct_def(&mut shared, "Result"),
        union_def(&mut shared, "Outcome", &["Result", "Ok"]),
        valspec_def(&mut shared, "f"),
    ]);
    let expected = resolve(&ast, &shared);
    let mut streamed = sail_resolve::resolve::Resolver::new();
    {
        let mut local = SimpleInterner::new();
        streamed.add_definition(&struct_def(&mut local, "Result"), &local);
    }
    {
        let mut local = SimpleInterner::new();
        streamed.add_definition(&union_def(&mut local, "Outcome", &["Result", "Ok"]), &local);
    }
    {
        let mut local = SimpleInterner::new();
        streamed.add_definition(&valspec_def(&mut local, "f"), &local);
    }
    assert_eq!(streamed.finish(), expected);
}

#[test]
fn collision_typename_equals_ctorname() {
    // Mirrors the Lean-backend failure: `union U { ... }` where a ctor
    // is also a type name.
    let mut it = SimpleInterner::new();
    let ast = Ast::new(vec![
        struct_def(&mut it, "Result"),
        union_def(&mut it, "Outcome", &["Result", "Ok"]),
    ]);
    let r = resolve(&ast, &it);
    assert!(r.errors.is_empty(), "collision must be non-fatal");
    assert_eq!(r.warnings.len(), 1);
    assert_eq!(r.warnings[0].name, "Result");
    assert_eq!(r.ctor_renames.get("Result"), Some(&"Result_ctor".to_owned()));
    assert!(!r.ctor_renames.contains_key("Ok"));
}

#[test]
fn self_collision_union_named_like_its_ctor() {
    let mut it = SimpleInterner::new();
    let ast = Ast::new(vec![union_def(&mut it, "U", &["U", "V"])]);
    let r = resolve(&ast, &it);
    assert!(r.errors.is_empty());
    assert_eq!(r.warnings.len(), 1);
    assert_eq!(r.ctor_renames.get("U"), Some(&"U_ctor".to_owned()));
}

#[test]
fn distinct_duplicate_type_is_error() {
    let mut it = SimpleInterner::new();
    let ast = Ast::new(vec![struct_def(&mut it, "U"), struct_def(&mut it, "U")]);
    let r = resolve(&ast, &it);
    assert_eq!(r.errors.len(), 1);
    assert_eq!(r.errors[0].name, "U");
    // First binding wins and stays queryable.
    assert!(r.env.lookup_type("U").is_some());
}

#[test]
fn distinct_errors_per_namespace() {
    let mut it = SimpleInterner::new();
    let ast = Ast::new(vec![
        valspec_def(&mut it, "f"),
        valspec_def(&mut it, "f"),
        register_def(&mut it, "r"),
        register_def(&mut it, "r"),
        mapping_def(&mut it, "m"),
        mapping_def(&mut it, "m"),
        union_def(&mut it, "U", &["A"]),
        union_def(&mut it, "V", &["A"]),
    ]);
    let r = resolve(&ast, &it);
    // Repeated val specs are a legacy warning; other duplicates remain errors.
    assert_eq!(r.errors.len(), 3, " {:?}", r.errors);
    assert_eq!(r.warnings.len(), 1);
    assert_eq!(r.warnings[0].kind, ResolveErrorKind::DuplicateValSpec);
    // Same name in *different* namespaces is fine: add a type `f`.
    let mut it2 = SimpleInterner::new();
    let ast2 = Ast::new(vec![struct_def(&mut it2, "f"), valspec_def(&mut it2, "f")]);
    let r2 = resolve(&ast2, &it2);
    assert!(r2.errors.is_empty(), " {:?}", r2.errors);
}

#[test]
fn repeated_valspecs_replace_in_source_order_and_preserve_externs() {
    let declaration = |it: &mut SimpleInterner, start| {
        let mut def = valspec_def(it, "f");
        let DefKind::Val(val) = &mut def.kind else { unreachable!() };
        val.span = Span { start, end: start + 5, ..Span::DUMMY };
        if start == 20 {
            val.extern_opt = Some(sail_ast::Extern {
                pure: true, bindings: vec![("lean".into(), "external_f".into())],
            });
        }
        def
    };
    let mut shared = SimpleInterner::new();
    let defs = [10, 20, 30].into_iter().map(|start| declaration(&mut shared, start)).collect();
    let expected = resolve(&Ast::new(defs), &shared);
    assert!(expected.is_ok(), "{:?}", expected.errors);
    let entry = expected.env.lookup_valspec("f").unwrap();
    assert_eq!(entry.span.start, 30);
    assert!(entry.has_extern);
    assert_eq!(expected.warnings.iter().map(|warning|
        (warning.first.start, warning.second.start)).collect::<Vec<_>>(),
        vec![(10, 20), (20, 30)]);

    // Shard-local symbols cannot change source-order replacement semantics.
    let mut streamed = sail_resolve::resolve::Resolver::new();
    for start in [10, 20, 30] {
        let mut local = SimpleInterner::new();
        local.intern(&format!("unrelated_{start}"));
        let def = declaration(&mut local, start);
        streamed.add_definition(&def, &local);
    }
    assert_eq!(streamed.finish(), expected);
}

#[test]
fn duplicate_ctor_across_unions_also_renamed() {
    let mut it = SimpleInterner::new();
    let ast = Ast::new(vec![union_def(&mut it, "A", &["Some"]), union_def(&mut it, "B", &["Some"])]);
    let r = resolve(&ast, &it);
    // Duplicate ctor is a hard error …
    assert_eq!(r.errors.len(), 1);
    // … and the rename map still covers it for downstream printing.
    assert_eq!(r.ctor_renames.get("Some"), Some(&"Some_ctor".to_owned()));
}

#[test]
fn order_independent_top_level() {
    // Use-before-def at top level must resolve (Sail is order-independent).
    let mut it = SimpleInterner::new();
    let ast = Ast::new(vec![union_def(&mut it, "V", &["U"]), struct_def(&mut it, "U")]);
    let r = resolve(&ast, &it);
    assert!(r.errors.is_empty(), " {:?}", r.errors);
    assert!(r.env.lookup_type("U").is_some());
    assert!(r.env.lookup_ctor("U").is_some());
}

#[test]
fn enum_members_resolve_as_ctors() {
    let mut it = SimpleInterner::new();
    let ast = Ast::new(vec![enum_def(&mut it, "E", &["X", "Y"])]);
    let r = resolve(&ast, &it);
    assert!(r.errors.is_empty(), " {:?}", r.errors);
    assert_eq!(r.env.lookup_ctor("X").map(|c| c.owner.as_str()), Some("E"));
}

#[test]
fn env_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<sail_resolve::Env>();
    assert_send_sync::<sail_resolve::resolve::ResolvedProgram>();
}

// --- Lane 78: full-resolver coverage (code only, E3: no run) ---------------

fn pat_id(it: &mut SimpleInterner, text: &str) -> sail_ast::Pat {
    Node::dummy(PatKind::Id(nm(it, text)))
}

fn arm_over(it: &mut SimpleInterner, binder: Option<&str>) -> PExp {
    let pat = match binder {
        Some(b) => pat_id(it, b),
        None => Node::dummy(PatKind::Wild),
    };
    PExp {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        kind: PExpKind::PatExp {
            pat,
            body: Node::dummy(ExpKind::Undef),
        },
    }
}

fn funcl(it: &mut SimpleInterner, fname: &str, binder: Option<&str>) -> Funcl {
    Funcl {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        name: nm(it, fname),
        arm: arm_over(it, binder),
    }
}

fn fundef(it: &mut SimpleInterner, fname: &str, binder: Option<&str>) -> Def {
    Node::dummy(DefKind::Fundef {
        rec: RecOpt::NonRec,
        annot: TAnnotOpt::None,
        funcls: vec![funcl(it, fname, binder)],
    })
}

fn scattered_open(it: &mut SimpleInterner, which: &str, name: &str) -> Def {
    let n = nm(it, name);
    let s = match which {
        "function" => ScatteredDef::Function { name: n, annot: TAnnotOpt::None },
        "mapping" => ScatteredDef::Mapping { name: n, annot: TAnnotOpt::None },
        "variant" => ScatteredDef::Variant { name: n, quant: Vec::new() },
        _ => ScatteredDef::Enum(n),
    };
    Node::dummy(DefKind::Scattered(s))
}

#[test]
fn fundef_binds_function_with_rec_flag_and_clause_count() {
    let mut it = SimpleInterner::new();
    let rec_def = Node::dummy(DefKind::Fundef {
        rec: RecOpt::Rec,
        annot: TAnnotOpt::None,
        funcls: vec![funcl(&mut it, "loop", Some("x")), funcl(&mut it, "loop", None)],
    });
    let ast = Ast::new(vec![rec_def, fundef(&mut it, "plain", None)]);
    let r = resolve(&ast, &it);
    assert!(r.errors.is_empty(), " {:?}", r.errors);
    let loop_e = r.env.lookup_function("loop").expect("loop bound");
    assert!(loop_e.is_rec);
    assert_eq!(loop_e.clauses, 2);
    let plain = r.env.lookup_function("plain").expect("plain bound");
    assert!(!plain.is_rec);
    assert_eq!(plain.clauses, 1);
}

#[test]
fn duplicate_function_is_error() {
    let mut it = SimpleInterner::new();
    let ast = Ast::new(vec![fundef(&mut it, "f", None), fundef(&mut it, "f", None)]);
    let r = resolve(&ast, &it);
    assert_eq!(r.errors.len(), 1);
    assert_eq!(r.errors[0].kind, ResolveErrorKind::DuplicateFunction);
}

#[test]
fn intra_clause_duplicate_pattern_is_error() {
    let mut it = SimpleInterner::new();
    let dup_pat: sail_ast::Pat = Node::dummy(PatKind::Tuple(vec![
        pat_id(&mut it, "x"),
        pat_id(&mut it, "x"),
    ]));
    let arm = PExp {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        kind: PExpKind::PatExp {
            pat: dup_pat,
            body: Node::dummy(ExpKind::Undef),
        },
    };
    let def = Node::dummy(DefKind::Fundef {
        rec: RecOpt::NonRec,
        annot: TAnnotOpt::None,
        funcls: vec![Funcl {
            id: NodeId::DUMMY,
            span: Span::DUMMY,
            name: nm(&mut it, "f"),
            arm,
        }],
    });
    let r = resolve(&Ast::new(vec![def]), &it);
    assert_eq!(r.errors.len(), 1);
    assert_eq!(r.errors[0].kind, ResolveErrorKind::DuplicateValue);
}

#[test]
fn overload_extensions_append_candidates_and_can_reference_overloads() {
    let mut it = SimpleInterner::new();
    let mut defs = vec![valspec_def(&mut it, "f"), valspec_def(&mut it, "g")];
    for (name, target) in [("o", "f"), ("o", "g"), ("nested", "o")] {
        defs.push(Node::dummy(DefKind::Overload {
            name: nm(&mut it, name), overloads: vec![nm(&mut it, target)],
        }));
    }
    let result = resolve(&Ast::new(defs), &it);
    assert!(result.is_ok(), "{:?}", result.errors);
    assert_eq!(result.env.lookup_overload("o").unwrap().targets, ["f", "g"]);
}

#[test]
fn overload_targets_must_exist_order_independent() {
    // Overload listed *before* its targets still resolves.
    let mut it = SimpleInterner::new();
    let over = Node::dummy(DefKind::Overload {
        name: nm(&mut it, "o"),
        overloads: vec![nm(&mut it, "f"), nm(&mut it, "g")],
    });
    let ast = Ast::new(vec![over, valspec_def(&mut it, "f"), fundef(&mut it, "g", None)]);
    let r = resolve(&ast, &it);
    assert!(r.errors.is_empty(), " {:?}", r.errors);
    assert_eq!(r.env.lookup_overload("o").map(|o| o.targets.len()), Some(2));

    let mut it2 = SimpleInterner::new();
    let bad = Node::dummy(DefKind::Overload {
        name: nm(&mut it2, "o"),
        overloads: vec![nm(&mut it2, "missing")],
    });
    let r2 = resolve(&Ast::new(vec![bad]), &it2);
    assert_eq!(r2.errors.len(), 1);
    assert_eq!(r2.errors[0].kind, ResolveErrorKind::UnknownOverloadTarget);
    assert_eq!(r2.errors[0].name, "missing");
}

#[test]
fn scattered_open_clause_impl_end_roundtrip() {
    let mut it = SimpleInterner::new();
    let ast = Ast::new(vec![
        scattered_open(&mut it, "function", "f"),
        Node::dummy(DefKind::Scattered(ScatteredDef::Funcl(funcl(&mut it, "f", Some(
            "x",
        ))))),
        Node::dummy(DefKind::Impl(funcl(&mut it, "f", None))),
        Node::dummy(DefKind::Scattered(ScatteredDef::End(nm(&mut it, "f")))),
    ]);
    let r = resolve(&ast, &it);
    assert!(r.errors.is_empty(), " {:?}", r.errors);
    let open = r.env.lookup_scattered(sail_resolve::ScatteredNamespace::Function, "f");
    assert!(open.is_some());
    assert!(open.map(|e| e.closed).unwrap_or(false));
    assert_eq!(r.env.impls.len(), 1);
}

#[test]
fn dangling_impl_clause_end_are_errors() {
    let mut it = SimpleInterner::new();
    let ast = Ast::new(vec![
        Node::dummy(DefKind::Impl(funcl(&mut it, "nope", None))),
        Node::dummy(DefKind::Scattered(ScatteredDef::Funcl(funcl(&mut it, "orphan", None)))),
        Node::dummy(DefKind::Scattered(ScatteredDef::End(nm(&mut it, "ghost")))),
    ]);
    let r = resolve(&ast, &it);
    assert_eq!(r.errors.len(), 3, " {:?}", r.errors);
    assert_eq!(r.errors[0].kind, ResolveErrorKind::DanglingImpl);
    assert_eq!(r.errors[1].kind, ResolveErrorKind::DanglingScatteredClause);
    assert_eq!(r.errors[2].kind, ResolveErrorKind::DanglingScatteredEnd);
}

#[test]
fn scattered_unioncl_ctor_collides_like_typedef_ctor() {
    let mut it = SimpleInterner::new();
    let unioncl = TypeUnion {
        id: NodeId::DUMMY,
        span: Span::DUMMY,
        typ: unknown_typ(),
        ctor: nm(&mut it, "U"),
    };
    let ast = Ast::new(vec![
        struct_def(&mut it, "U"),
        scattered_open(&mut it, "variant", "V"),
        Node::dummy(DefKind::Scattered(ScatteredDef::Unioncl {
            name: nm(&mut it, "V"),
            union: unioncl,
        })),
    ]);
    let r = resolve(&ast, &it);
    assert!(r.errors.is_empty(), " {:?}", r.errors);
    assert_eq!(r.warnings.len(), 1);
    assert_eq!(r.ctor_renames.get("U"), Some(&"U_ctor".to_owned()));
}

#[test]
fn scattered_enumcl_duplicate_across_owners_renamed() {
    let mut it = SimpleInterner::new();
    let ast = Ast::new(vec![
        scattered_open(&mut it, "enum", "A"),
        scattered_open(&mut it, "enum", "B"),
        Node::dummy(DefKind::Scattered(ScatteredDef::Enumcl {
            enum_name: nm(&mut it, "A"),
            member: nm(&mut it, "Same"),
        })),
        Node::dummy(DefKind::Scattered(ScatteredDef::Enumcl {
            enum_name: nm(&mut it, "B"),
            member: nm(&mut it, "Same"),
        })),
    ]);
    let r = resolve(&ast, &it);
    assert_eq!(r.errors.len(), 1);
    assert_eq!(r.errors[0].kind, ResolveErrorKind::DuplicateCtor);
    assert_eq!(r.ctor_renames.get("Same"), Some(&"Same_ctor".to_owned()));
}

#[test]
fn top_level_let_binds_values() {
    let mut it = SimpleInterner::new();
    let ast = Ast::new(vec![
        Node::dummy(DefKind::Let {
            pat: pat_id(&mut it, "v"),
            exp: Node::dummy(ExpKind::Undef),
        }),
        Node::dummy(DefKind::Let {
            pat: pat_id(&mut it, "v"),
            exp: Node::dummy(ExpKind::Undef),
        }),
    ]);
    let r = resolve(&ast, &it);
    assert_eq!(r.errors.len(), 1);
    assert_eq!(r.errors[0].kind, ResolveErrorKind::DuplicateValue);
    assert!(r.env.lookup_value("v").is_some());
}

#[test]
fn meta_defs_recorded_without_errors() {
    let mut it = SimpleInterner::new();
    let ast = Ast::new(vec![
        Node::dummy(DefKind::Fixity {
            fixity: Fixity::InfixL,
            level: "6".to_owned(),
            id: nm(&mut it, "++"),
        }),
        Node::dummy(DefKind::Default(Order::Inc)),
        Node::dummy(DefKind::Pragma {
            key: "file_start".to_owned(),
            payload: "mem.sail".to_owned(),
        }),
        Node::dummy(DefKind::Outcome {
            name: nm(&mut it, "outcome"),
            defs: Vec::new(),
            typschm: TypSchm { quant: Vec::new(), typ: unknown_typ() },
            quant: Vec::new(),
        }),
        Node::dummy(DefKind::Instantiation {
            name: nm(&mut it, "inst"),
            substs: Vec::new(),
        }),
        Node::dummy(DefKind::Measure {
            name: nm(&mut it, "m"),
            pat: Node::dummy(PatKind::Wild),
            exp: Node::dummy(ExpKind::Undef),
        }),
        Node::dummy(DefKind::LoopMeasures {
            name: nm(&mut it, "lm"),
            measures: vec![(Loop::While, Node::dummy(ExpKind::Undef))],
        }),
        Node::dummy(DefKind::InternalMutrec(vec![FunclSet {
            rec: RecOpt::NonRec,
            annot: TAnnotOpt::None,
            funcls: vec![funcl(&mut it, "mr", None)],
        }])),
        Node::dummy(DefKind::Constraint(Node::dummy(ConstraintKind::True))),
    ]);
    let r = resolve(&ast, &it);
    assert!(r.errors.is_empty(), " {:?}", r.errors);
    assert!(r.env.lookup_fixity("++").is_some());
    assert!(r.env.default.is_some());
    assert_eq!(r.env.pragmas.len(), 1);
    assert!(r.env.lookup_outcome("outcome").is_some());
    assert!(r.env.lookup_instantiation("inst").is_some());
    assert!(r.env.lookup_measure("m").is_some());
    assert!(r.env.lookup_measure("lm").is_some());
    assert!(r.env.lookup_function("mr").is_some());
    assert_eq!(r.env.mutrec_groups, 1);
    assert_eq!(r.env.constraints.len(), 1);
}

#[test]
fn duplicate_default_is_error() {
    let mut it = SimpleInterner::new();
    let ast = Ast::new(vec![
        Node::dummy(DefKind::Default(Order::Inc)),
        Node::dummy(DefKind::Default(Order::Dec)),
    ]);
    let r = resolve(&ast, &it);
    assert_eq!(r.errors.len(), 1);
    assert_eq!(r.errors[0].kind, ResolveErrorKind::DuplicateDefault);
}

#[test]
fn namespaces_stay_separate_across_all_kinds() {
    let mut it = SimpleInterner::new();
    let ast = Ast::new(vec![
        struct_def(&mut it, "n"),
        valspec_def(&mut it, "n"),
        fundef(&mut it, "n", None),
        register_def(&mut it, "n"),
        mapping_def(&mut it, "n"),
        Node::dummy(DefKind::Let {
            pat: pat_id(&mut it, "n"),
            exp: Node::dummy(ExpKind::Undef),
        }),
    ]);
    let r = resolve(&ast, &it);
    assert!(r.errors.is_empty(), " {:?}", r.errors);
}

#[test]
fn pat_names_skips_struct_references() {
    let mut it = SimpleInterner::new();
    let field_name = nm(&mut it, "field");
    let pat: sail_ast::Pat = Node::dummy(PatKind::Struct {
        name: sail_ast::StructName::Id(nm(&mut it, "SomeStruct")),
        fields: vec![(field_name, pat_id(&mut it, "x"))],
        wildcard: false,
    });
    let names = sail_resolve::resolve::pat_names(&pat, &it);
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].0, "x");
}
