//! Lane 41: Vektor-LHS-Brücke (`vector_lexp_bridge`).
//!
//! Prüft die MiniTyp→Type-Brücke gegen die gemergte Lane-07/30-Welt:
//! Index-Nexp aus `Exp` synthetisieren, kanonische Typknoten nach `MiniTyp`
//! heben, `MiniTyp`-Ergebnisse nach `Typ` heben; Lücken sind `Unsupported`.
//! Beweise laufen über [`StubSolver`]; der kanonische Hook
//! (`patch-vector-lexp.diff`, `program.prove`) nutzt dieselbe Reihenfolge:
//! Wurzel expandieren, Index inferieren, Bounds zuerst, dann RHS.
use sail_ast::{
    Exp, ExpKind, Interner, LitKind, Name, Node, SimpleInterner, Span, Typ, TypArgKind, TypKind,
};
use sail_ty::{
    error::TypeErrorKind,
    lexp_bounds::VectorRoot,
    nexp::{MiniTyp, Nexp},
    solver::StubSolver,
    vector_lexp::{check_assign, infer_vector_elem},
    vector_lexp_bridge::{
        ast_of_mini_typ, index_nexp_from_exp, mini_typ_of_canonical, vector_root_of_canonical,
    },
};

fn mk_name(it: &mut impl Interner, text: &str) -> Name {
    Name::dummy(it.intern(text))
}

fn mk_id_exp(it: &mut impl Interner, name: &str) -> Exp {
    Node::dummy(ExpKind::Id(mk_name(it, name)))
}

fn mk_num_exp(it: &mut impl Interner, value: &str) -> Exp {
    let sym = it.intern(value);
    Node::dummy(ExpKind::Lit(Node::dummy(LitKind::Num(sym))))
}

fn mk_int_typ(it: &mut impl Interner) -> Typ {
    Node::dummy(TypKind::Id(mk_name(it, "int")))
}

fn mk_vector_typ(it: &mut impl Interner, len: &str, elem: Typ) -> Typ {
    let name = mk_name(it, "vector");
    let len_arg: sail_ast::TypArg = Node::dummy(TypArgKind::NExp(Node::dummy(
        sail_ast::NExpKind::Constant(len.to_owned()),
    )));
    let elem_arg: sail_ast::TypArg = Node::dummy(TypArgKind::Typ(elem));
    Node::dummy(TypKind::App {
        name,
        args: vec![len_arg, elem_arg],
    })
}

fn mk_bitvector_typ(it: &mut impl Interner, len: &str) -> Typ {
    let name = mk_name(it, "bitvector");
    let len_arg: sail_ast::TypArg = Node::dummy(TypArgKind::NExp(Node::dummy(
        sail_ast::NExpKind::Constant(len.to_owned()),
    )));
    Node::dummy(TypKind::App {
        name,
        args: vec![len_arg],
    })
}

#[test]
fn full_index_pipeline_const_proves_and_lifts_element() {
    // `v[3]` mit `v : vector(8, int)`: Wurzel -> Index-Nexp -> MiniTyp ->
    // Bounds-Beweis (StubSolver) -> Lift nach Typ.
    let mut it = SimpleInterner::new();
    let elem = mk_int_typ(&mut it);
    let root_typ = mk_vector_typ(&mut it, "8", elem);
    let root = vector_root_of_canonical(&root_typ, &it, Span::DUMMY).unwrap();
    assert_eq!(
        root,
        VectorRoot::Vector {
            len: Nexp::Const(8),
            elem: MiniTyp::Int,
        }
    );

    let index_exp = mk_num_exp(&mut it, "3");
    let index_nexp = index_nexp_from_exp(&index_exp, &it).unwrap();
    assert_eq!(index_nexp, Nexp::Const(3));

    let index_typ_ast = mk_int_typ(&mut it);
    let index_typ = mini_typ_of_canonical(&index_typ_ast, &it, Span::DUMMY).unwrap();
    assert_eq!(index_typ, MiniTyp::Int);

    let elem = infer_vector_elem(
        &StubSolver::new(),
        &root,
        &index_nexp,
        &index_typ,
        Span::DUMMY,
    )
    .unwrap();
    assert_eq!(elem, MiniTyp::Int);

    let lifted = ast_of_mini_typ(&elem, &mut it);
    let back = sail_ty::conv::typ_from_ast(&lifted, &it).expect("lift roundtrips");
    assert_eq!(back, MiniTyp::Int);
}

#[test]
fn full_index_pipeline_out_of_bounds_is_constraint_not_proved() {
    let mut it = SimpleInterner::new();
    let elem = mk_int_typ(&mut it);
    let root_typ = mk_vector_typ(&mut it, "8", elem);
    let root = vector_root_of_canonical(&root_typ, &it, Span::DUMMY).unwrap();
    let index_nexp = index_nexp_from_exp(&mk_num_exp(&mut it, "8"), &it).unwrap();
    let err = infer_vector_elem(
        &StubSolver::new(),
        &root,
        &index_nexp,
        &MiniTyp::Int,
        Span::DUMMY,
    )
    .unwrap_err();
    assert!(matches!(err.kind, TypeErrorKind::ConstraintNotProved(_)));
}

#[test]
fn bitvector_index_yields_bit_and_lifts() {
    let mut it = SimpleInterner::new();
    let root_typ = mk_bitvector_typ(&mut it, "4");
    let root = vector_root_of_canonical(&root_typ, &it, Span::DUMMY).unwrap();
    let elem = infer_vector_elem(
        &StubSolver::new(),
        &root,
        &Nexp::Const(0),
        &MiniTyp::Int,
        Span::DUMMY,
    )
    .unwrap();
    assert_eq!(elem, MiniTyp::Bit);
    let lifted = ast_of_mini_typ(&elem, &mut it);
    let back = sail_ty::conv::typ_from_ast(&lifted, &it).expect("bit lifts");
    assert_eq!(back, MiniTyp::Bit);
}

#[test]
fn variable_index_synthesizes_to_var() {
    let mut it = SimpleInterner::new();
    let exp = mk_id_exp(&mut it, "i");
    assert_eq!(
        index_nexp_from_exp(&exp, &it).unwrap(),
        Nexp::Var("i".to_owned())
    );
}

#[test]
fn non_index_expression_is_unsupported_never_guessed() {
    let mut it = SimpleInterner::new();
    let app: Exp = Node::dummy(ExpKind::App {
        func: mk_name(&mut it, "add"),
        args: vec![mk_id_exp(&mut it, "i")],
    });
    let err = index_nexp_from_exp(&app, &it).unwrap_err();
    assert!(matches!(err.kind, TypeErrorKind::Unsupported(_)));
}

#[test]
fn non_vector_root_is_invalid_assignment() {
    let mut it = SimpleInterner::new();
    let int = mk_int_typ(&mut it);
    let err = vector_root_of_canonical(&int, &it, Span::DUMMY).unwrap_err();
    assert!(
        matches!(err.kind, TypeErrorKind::Unsupported(ref msg) if msg == "Invalid vector assignment l-expression"),
        "unexpected {err:?}"
    );
}

#[test]
fn lexp_error_wins_over_rhs() {
    // `check_assign`-Priorität: Bounds-Fehler zuerst, RHS-Ergebnis egal.
    let lexp = infer_vector_elem(
        &StubSolver::new(),
        &VectorRoot::Vector {
            len: Nexp::Const(8),
            elem: MiniTyp::Int,
        },
        &Nexp::Const(8),
        &MiniTyp::Int,
        Span::DUMMY,
    );
    assert!(lexp.is_err());
    let err = check_assign(lexp, Ok(()), Span::DUMMY).unwrap_err();
    assert!(matches!(err.kind, TypeErrorKind::ConstraintNotProved(_)));
}
