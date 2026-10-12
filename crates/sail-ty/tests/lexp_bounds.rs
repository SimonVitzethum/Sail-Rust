//! Lane 07: Vektor-LHS-Bounds (`LE_vector` / `LE_vector_range` nach
//! OCaml `type_check.ml::infer_lexp`). Reine `Mini`-Ebene + [`StubSolver`];
//! keine `canonical_body`/`program`-Aenderung.
use sail_ty::{
    error::{Span, TypeErrorKind},
    lexp_bounds::{
        RootBinding, VectorRoot, check_assign, check_index_typ, check_root, check_slice_root_len,
        check_vector_index, check_vector_slice, element_typ, index_obligation, slice_obligation,
        HOOK_NOTE,
    },
    nexp::{MiniOrder, MiniTyp, NConstraint, Nexp},
    solver::{ProveResult, Solver, StubSolver},
};

fn solver() -> StubSolver {
    StubSolver::new()
}

fn vector_root(len: i128, elem: MiniTyp) -> VectorRoot {
    VectorRoot::Vector {
        len: Nexp::Const(len),
        elem,
    }
}

fn bitvector_root(len: i128) -> VectorRoot {
    VectorRoot::Bitvector {
        len: Nexp::Const(len),
    }
}

#[test]
fn index_obligation_shape_is_zero_le_index_and_index_lt_len() {
    let obligation = index_obligation(&Nexp::Var("i".into()), &Nexp::Var("len".into()));
    let expected = NConstraint::And(
        Box::new(NConstraint::Ge(
            Box::new(Nexp::Var("i".into())),
            Box::new(Nexp::Const(0)),
        )),
        Box::new(NConstraint::Lt(
            Box::new(Nexp::Var("i".into())),
            Box::new(Nexp::Var("len".into())),
        )),
    );
    assert_eq!(obligation, expected);
}

#[test]
fn const_index_in_bounds_proves_element_typ() {
    let root = vector_root(8, MiniTyp::Int);
    let got = check_vector_index(
        &solver(),
        &root,
        &Nexp::Const(3),
        &MiniTyp::Int,
        Span::DUMMY,
    )
    .unwrap();
    assert_eq!(got, MiniTyp::Int);
}

#[test]
fn const_index_out_of_bounds_is_constraint_not_proved() {
    let root = vector_root(8, MiniTyp::Int);
    let err = check_vector_index(
        &solver(),
        &root,
        &Nexp::Const(8),
        &MiniTyp::Int,
        Span::DUMMY,
    )
    .unwrap_err();
    assert!(matches!(
        err.kind,
        TypeErrorKind::ConstraintNotProved(_)
    ));
}

#[test]
fn symbolic_index_is_unknown_and_stays_sound() {
    // Weder beweisbar noch widerlegbar: StubSolver meldet Unknown ...
    let obligation = index_obligation(&Nexp::Var("i".into()), &Nexp::Const(8));
    assert_eq!(solver().prove(&obligation), ProveResult::Unknown);
    // ... und check_vector_index fabriziert daraus keinen Beweis.
    let root = vector_root(8, MiniTyp::Int);
    let err = check_vector_index(
        &solver(),
        &root,
        &Nexp::Var("i".into()),
        &MiniTyp::Int,
        Span::DUMMY,
    )
    .unwrap_err();
    assert!(matches!(
        err.kind,
        TypeErrorKind::ConstraintNotProved(_)
    ));
}

#[test]
fn bitvector_index_yields_bit() {
    let root = bitvector_root(4);
    let got = check_vector_index(
        &solver(),
        &root,
        &Nexp::Const(0),
        &MiniTyp::Range(Box::new(Nexp::Const(0)), Box::new(Nexp::Const(7))),
        Span::DUMMY,
    )
    .unwrap();
    assert_eq!(got, MiniTyp::Bit);
    assert_eq!(element_typ(&root), MiniTyp::Bit);
}

#[test]
fn vector_index_yields_element_typ() {
    let root = vector_root(2, MiniTyp::Bool);
    assert_eq!(element_typ(&root), MiniTyp::Bool);
}

#[test]
fn slice_inc_proves_and_computes_len() {
    // v[2 .. 5], inc, len 8: slice_len = (5-2)+1 = 4, alles konstant beweisbar.
    let root = bitvector_root(8);
    let got = check_vector_slice(
        &solver(),
        &root,
        &Nexp::Const(2),
        &MiniTyp::Int,
        &Nexp::Const(5),
        &MiniTyp::Int,
        MiniOrder::Inc,
        Span::DUMMY,
    )
    .unwrap();
    let MiniTyp::Bits(len) = &got else {
        panic!("expected Bits slice type, got {got:?}");
    };
    // Die Slice-Laenge bleibt symbolisch `(second - first) + 1` (wie OCaml
    // vor `nexp_simp`); konstant faltbar muss sie 4 ergeben.
    assert_eq!(len.as_const(), Some(4));
}

#[test]
fn slice_dec_proves_and_computes_len() {
    // v[5 .. 2], dec, len 8: slice_len = (5-2)+1 = 4.
    let root = bitvector_root(8);
    let got = check_vector_slice(
        &solver(),
        &root,
        &Nexp::Const(5),
        &MiniTyp::Int,
        &Nexp::Const(2),
        &MiniTyp::Int,
        MiniOrder::Dec,
        Span::DUMMY,
    )
    .unwrap();
    let MiniTyp::Bits(len) = &got else {
        panic!("expected Bits slice type, got {got:?}");
    };
    assert_eq!(len.as_const(), Some(4));
}

#[test]
fn slice_bounds_are_order_dependent() {
    // Dieselben Grenzen (first=5, second=2, len 8): unter dec beweisbar ...
    let (dec_len, dec_check) = slice_obligation(
        &Nexp::Const(5),
        &Nexp::Const(2),
        &Nexp::Const(8),
        MiniOrder::Dec,
    );
    assert_eq!(dec_len.as_const(), Some(4));
    assert_eq!(solver().prove(&dec_check), ProveResult::Proved);
    // ... unter inc widerlegt (5 <= 2 ist falsch).
    let (inc_len, inc_check) = slice_obligation(
        &Nexp::Const(5),
        &Nexp::Const(2),
        &Nexp::Const(8),
        MiniOrder::Inc,
    );
    assert_eq!(inc_len.as_const(), Some(-2));
    assert_eq!(solver().prove(&inc_check), ProveResult::Refuted);
}

#[test]
fn slice_on_vector_root_is_rejected() {
    let root = vector_root(8, MiniTyp::Int);
    let err = check_slice_root_len(&root, Span::DUMMY).unwrap_err();
    assert!(matches!(err.kind, TypeErrorKind::Mismatch { .. }));
    // End-to-end: Slice gegen Vektor-Wurzel scheitert vor jedem Beweis.
    let err = check_vector_slice(
        &solver(),
        &root,
        &Nexp::Const(0),
        &MiniTyp::Int,
        &Nexp::Const(1),
        &MiniTyp::Int,
        MiniOrder::Inc,
        Span::DUMMY,
    )
    .unwrap_err();
    assert!(matches!(err.kind, TypeErrorKind::Mismatch { .. }));
}

#[test]
fn immutable_and_unbound_roots_are_rejected() {
    let err = check_root("x", &RootBinding::Immutable, Span::DUMMY).unwrap_err();
    assert!(matches!(err.kind, TypeErrorKind::RegisterError(_)));
    let err = check_root("fresh", &RootBinding::Unbound, Span::DUMMY).unwrap_err();
    assert!(matches!(err.kind, TypeErrorKind::UnboundVar(_)));
}

#[test]
fn mutable_and_register_roots_pass_through() {
    let root = bitvector_root(8);
    let got = check_root("v", &RootBinding::Mutable(root.clone()), Span::DUMMY).unwrap();
    assert_eq!(got, root);
    let got = check_root("r", &RootBinding::Register(root.clone()), Span::DUMMY).unwrap();
    assert_eq!(got, root);
}

#[test]
fn non_int_index_typ_is_rejected_before_any_proof() {
    // Indexausdruecke werden vollstaendig geprueft: ein bool-Typ scheitert
    // mit Mismatch, nicht erst am Bounds-Beweis.
    let err = check_index_typ(&MiniTyp::Bool, Span::DUMMY).unwrap_err();
    assert!(matches!(err.kind, TypeErrorKind::Mismatch { .. }));
    let root = vector_root(8, MiniTyp::Int);
    let err = check_vector_index(
        &solver(),
        &root,
        &Nexp::Const(3),
        &MiniTyp::Bool,
        Span::DUMMY,
    )
    .unwrap_err();
    assert!(matches!(err.kind, TypeErrorKind::Mismatch { .. }));
}

#[test]
fn rhs_inference_cannot_bypass_index_errors() {
    // LExp-Fehler gewinnt: selbst bei gelingender RHS-Inferenz faellt Assign.
    let lexp: sail_ty::error::TypeResult<MiniTyp> = Err(TypeErrorKind::ConstraintNotProved(
        "vector index bounds".into(),
    ))
    .pipe_span();
    let rhs: sail_ty::error::TypeResult<()> = Ok(());
    let err = check_assign(lexp, rhs, Span::DUMMY).unwrap_err();
    assert!(matches!(
        err.kind,
        TypeErrorKind::ConstraintNotProved(_)
    ));
    // LExp-Fehler gewinnt auch gegen einen RHS-Fehler (kein Verschlucken
    // in die andere Richtung, kein Ueberschreiben durch RHS-Gelingen).
    let lexp: sail_ty::error::TypeResult<MiniTyp> = Err(TypeErrorKind::ConstraintNotProved(
        "vector index bounds".into(),
    ))
    .pipe_span();
    let rhs: sail_ty::error::TypeResult<()> = Err(TypeErrorKind::Mismatch {
        expected: "int".into(),
        found: "bool".into(),
    })
    .pipe_span();
    let err = check_assign(lexp, rhs, Span::DUMMY).unwrap_err();
    assert!(matches!(
        err.kind,
        TypeErrorKind::ConstraintNotProved(_)
    ));
    // Umgekehrt: gute LExp + schlechte RHS => RHS-Fehler kommt durch.
    let lexp: sail_ty::error::TypeResult<MiniTyp> = Ok(MiniTyp::Int);
    let rhs: sail_ty::error::TypeResult<()> = Err(TypeErrorKind::Mismatch {
        expected: "int".into(),
        found: "bool".into(),
    })
    .pipe_span();
    let err = check_assign(lexp, rhs, Span::DUMMY).unwrap_err();
    assert!(matches!(err.kind, TypeErrorKind::Mismatch { .. }));
}

/// Kleine Test-Hilfe: `TypeErrorKind` mit Dummy-Span in `TypeResult` heben.
trait PipeSpan {
    type Out;
    fn pipe_span(self) -> Self::Out;
}

impl<T> PipeSpan for Result<T, TypeErrorKind> {
    type Out = sail_ty::error::TypeResult<T>;
    fn pipe_span(self) -> Self::Out {
        self.map_err(|kind| sail_ty::error::TypeError {
            span: Span::DUMMY,
            kind,
        })
    }
}

#[test]
fn hook_note_names_the_canonical_body_wiring_point() {
    assert!(HOOK_NOTE.contains("check_vector_index"));
    assert!(HOOK_NOTE.contains("check_vector_slice"));
}
