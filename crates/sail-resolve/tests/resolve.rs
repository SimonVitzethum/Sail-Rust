//! Integration tests: collision case + happy path + distincto errors.

use sail_resolve::mini_ast::{
    Ast, CtorDecl, Def, MappingDef, RegisterDef, Span, TypeDef, TypeDefKind, ValSpec,
};
use sail_resolve::resolve::resolve;

const S: Span = Span::new(1, 1);
const S2: Span = Span::new(2, 5);

fn union_def(owner: &str, ctors: &[&str]) -> Def {
    Def::Type(TypeDef::new(
        owner,
        TypeDefKind::Union(
            ctors
                .iter()
                .map(|c| CtorDecl::new((*c).to_owned(), S))
                .collect(),
        ),
        S,
    ))
}

#[test]
fn happy_path_no_errors_no_renames() {
    let ast = Ast::new(vec![
        Def::Type(TypeDef::new("U", TypeDefKind::Struct, S)),
        union_def("V", &["A", "B"]),
        Def::Type(TypeDef::new(
            "E",
            TypeDefKind::Enum(vec![CtorDecl::new("X", S), CtorDecl::new("Y", S)]),
            S,
        )),
        Def::Type(TypeDef::new("N", TypeDefKind::Abbrev, S)),
        Def::ValSpec(ValSpec::new("f", S)),
        Def::Register(RegisterDef::new("sp", S)),
        Def::Mapping(MappingDef::new("decode", S)),
    ]);
    let r = resolve(&ast);
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
    let ast = Ast::new(vec![
        Def::Type(TypeDef::new("Result", TypeDefKind::Struct, S)),
        union_def("Outcome", &["Result", "Ok"]),
    ]);
    let r = resolve(&ast);
    assert!(r.errors.is_empty(), "collision must be non-fatal");
    assert_eq!(r.warnings.len(), 1);
    assert_eq!(r.warnings[0].name, "Result");
    assert_eq!(
        r.ctor_renames.get("Result"),
        Some(&"Result_ctor".to_owned())
    );
    assert!(!r.ctor_renames.contains_key("Ok"));
}

#[test]
fn self_collision_union_named_like_its_ctor() {
    let ast = Ast::new(vec![union_def("U", &["U", "V"])]);
    let r = resolve(&ast);
    assert!(r.errors.is_empty());
    assert_eq!(r.warnings.len(), 1);
    assert_eq!(r.ctor_renames.get("U"), Some(&"U_ctor".to_owned()));
}

#[test]
fn distinct_duplicate_type_is_error() {
    let ast = Ast::new(vec![
        Def::Type(TypeDef::new("U", TypeDefKind::Struct, S)),
        Def::Type(TypeDef::new("U", TypeDefKind::Struct, S2)),
    ]);
    let r = resolve(&ast);
    assert_eq!(r.errors.len(), 1);
    assert_eq!(r.errors[0].name, "U");
    // First binding wins and stays queryable.
    assert_eq!(r.env.lookup_type("U").map(|t| t.span), Some(S));
}

#[test]
fn distinct_errors_per_namespace() {
    let ast = Ast::new(vec![
        Def::ValSpec(ValSpec::new("f", S)),
        Def::ValSpec(ValSpec::new("f", S2)),
        Def::Register(RegisterDef::new("r", S)),
        Def::Register(RegisterDef::new("r", S2)),
        Def::Mapping(MappingDef::new("m", S)),
        Def::Mapping(MappingDef::new("m", S2)),
        union_def("U", &["A"]),
        union_def("V", &["A"]),
    ]);
    let r = resolve(&ast);
    // valspec + register + mapping + ctor duplicates = 4 errors.
    assert_eq!(r.errors.len(), 4, " {:?}", r.errors);
    // Same name in *different* namespaces is fine: add a type `f`.
    let ast2 = Ast::new(vec![
        Def::Type(TypeDef::new("f", TypeDefKind::Struct, S)),
        Def::ValSpec(ValSpec::new("f", S)),
    ]);
    let r2 = resolve(&ast2);
    assert!(r2.errors.is_empty(), " {:?}", r2.errors);
}

#[test]
fn duplicate_ctor_across_unions_also_renamed() {
    let ast = Ast::new(vec![union_def("A", &["Some"]), union_def("B", &["Some"])]);
    let r = resolve(&ast);
    // Duplicate ctor is a hard error …
    assert_eq!(r.errors.len(), 1);
    // … and the rename map still covers it for downstream printing.
    assert_eq!(r.ctor_renames.get("Some"), Some(&"Some_ctor".to_owned()));
}

#[test]
fn order_independent_top_level() {
    // Use-before-def at top level must resolve (Sail is order-independent).
    let ast = Ast::new(vec![
        union_def("V", &["U"]),
        Def::Type(TypeDef::new("U", TypeDefKind::Struct, S2)),
    ]);
    let r = resolve(&ast);
    assert!(r.errors.is_empty(), " {:?}", r.errors);
    assert!(r.env.lookup_type("U").is_some());
    assert!(r.env.lookup_ctor("U").is_some());
}

#[test]
fn env_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<sail_resolve::Env>();
    assert_send_sync::<sail_resolve::resolve::ResolvedProgram>();
}
