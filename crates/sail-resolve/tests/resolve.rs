//! Integration tests: collision case + happy path + distincto errors.
//!
//! Fixtures build canonical `sail-ast` nodes directly (Lane 61, #9–#16)
//! with a `SimpleInterner` for names; all spans are `DUMMY`.

use sail_ast::{
    Ast, DecSpec, Def, DefKind, Interner, Mapcl, Name, Node, NodeId, SimpleInterner, Span,
    TAnnotOpt, Typ, TypKind, TypSchm, TypeDefKind, TypeUnion, ValSpec,
};
use sail_resolve::resolve::resolve;

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
    // valspec + register + mapping + ctor duplicates = 4 errors.
    assert_eq!(r.errors.len(), 4, " {:?}", r.errors);
    // Same name in *different* namespaces is fine: add a type `f`.
    let mut it2 = SimpleInterner::new();
    let ast2 = Ast::new(vec![struct_def(&mut it2, "f"), valspec_def(&mut it2, "f")]);
    let r2 = resolve(&ast2, &it2);
    assert!(r2.errors.is_empty(), " {:?}", r2.errors);
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
