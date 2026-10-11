//! Unit tests: `enum.sail` / `option.sail` / `struct.sail` shapes plus the
//! `bitfield.sail` error case, all built directly as [`sail_ast`] nodes.

use super::*;
use sail_ast::{
    Constraint, FileId, KOpt, Name, Node, NodeId, SimpleInterner, TypeUnion,
};

fn span() -> Span {
    Span::new(FileId(0), 0, 1)
}

fn name(ix: &mut SimpleInterner, text: &str) -> Name {
    Name::new(NodeId::DUMMY, span(), ix.intern(text), false)
}

fn kid(ix: &mut SimpleInterner, text: &str) -> Symbol {
    ix.intern(text)
}

fn typ_id(ix: &mut SimpleInterner, text: &str) -> Typ {
    Node::dummy(TypKind::Id(name(ix, text)))
}

fn typ_var(k: Symbol) -> Typ {
    Node::dummy(TypKind::Var(k))
}

fn targ_typ(t: Typ) -> TypArg {
    Node::dummy(TypArgKind::Typ(t))
}

fn targ_nexp(n: NExp) -> TypArg {
    Node::dummy(TypArgKind::NExp(n))
}

fn nexp_var(k: Symbol) -> NExp {
    Node::dummy(NExpKind::Var(k))
}

fn nexp_const(c: &str) -> NExp {
    Node::dummy(NExpKind::Constant(c.to_string()))
}

fn typ_app(ix: &mut SimpleInterner, head: &str, args: Vec<TypArg>) -> Typ {
    Node::dummy(TypKind::App {
        name: name(ix, head),
        args,
    })
}

fn qid(kind: Kind, k: Symbol) -> QuantItem {
    QuantItem::Id(KOpt { kind, kid: k })
}

fn printer() -> TypedefPrinter {
    TypedefPrinter::with_default()
}

fn render(td: &TypeDef, ix: &SimpleInterner) -> (String, Printed) {
    printer().render_typedef(td, ix).expect("must render")
}

// ---------------------------------------------------------------------------
// enum.sail: `enum E = A | B | C`
// ---------------------------------------------------------------------------

#[test]
fn enum_block_matches_expected_lean() {
    let mut ix = SimpleInterner::new();
    let td = Node::dummy(TypeDefKind::Enum {
        name: name(&mut ix, "E"),
        members: vec![name(&mut ix, "A"), name(&mut ix, "B"), name(&mut ix, "C")],
        wf: false,
    });
    let (got, printed) = render(&td, &ix);
    assert_eq!(
        got,
        "inductive E where | A | B | C\n  deriving BEq, Inhabited, Repr\n  open E\n"
    );
    assert_eq!(printed.opened.as_deref(), Some("E"));
}

#[test]
fn empty_enum_derives_beq_repr_only() {
    let mut ix = SimpleInterner::new();
    let td = Node::dummy(TypeDefKind::Enum {
        name: name(&mut ix, "Empty"),
        members: vec![],
        wf: false,
    });
    let (got, _) = render(&td, &ix);
    assert_eq!(
        got,
        "inductive Empty where\n  deriving BEq, Repr\n  open Empty\n"
    );
}

#[test]
fn non_beq_types_skip_beq() {
    let mut ix = SimpleInterner::new();
    let td = Node::dummy(TypeDefKind::Enum {
        name: name(&mut ix, "E"),
        members: vec![name(&mut ix, "A")],
        wf: false,
    });
    let p = TypedefPrinter::new(Options {
        non_beq_types: BTreeSet::from(["E".to_string()]),
    });
    let (got, _) = p.render_typedef(&td, &ix).unwrap();
    assert_eq!(
        got,
        "inductive E where | A\n  deriving Inhabited, Repr\n  open E\n"
    );
}

// ---------------------------------------------------------------------------
// option.sail prelude shape: `union option('a : Type) = Some('a) | None(unit)`
// ---------------------------------------------------------------------------

fn option_typedef(ix: &mut SimpleInterner) -> TypeDef {
    let ka = kid(ix, "'a");
    let some_ty = typ_var(ka);
    let none_ty = typ_id(ix, "unit");
    Node::dummy(TypeDefKind::Variant {
        name: name(ix, "option"),
        quant: vec![qid(Kind::Type, ka)],
        ctors: vec![
            TypeUnion {
                id: NodeId::DUMMY,
                span: span(),
                typ: some_ty,
                ctor: name(ix, "Some"),
            },
            TypeUnion {
                id: NodeId::DUMMY,
                span: span(),
                typ: none_ty,
                ctor: name(ix, "None"),
            },
        ],
        wf: false,
    })
}

#[test]
fn option_variant_matches_expected_lean() {
    let mut ix = SimpleInterner::new();
    let td = option_typedef(&mut ix);
    let (got, printed) = render(&td, &ix);
    assert_eq!(
        got,
        "/-- Type quantifiers: k_a : Type -/\n\
         inductive option (k_a : Type) where\n\
         \x20 | Some (_ : k_a)\n\
         \x20 | None (_ : Unit)\n\
         \x20 deriving Inhabited, BEq, Repr\n\
         \x20 open option\n"
    );
    assert_eq!(printed.opened.as_deref(), Some("option"));
}

#[test]
fn empty_variant_skips_inhabited() {
    let mut ix = SimpleInterner::new();
    let td = Node::dummy(TypeDefKind::Variant {
        name: name(&mut ix, "Void"),
        quant: vec![],
        ctors: vec![],
        wf: false,
    });
    let (got, _) = render(&td, &ix);
    assert_eq!(
        got,
        "inductive Void where\n  deriving BEq, Repr\n  open Void\n"
    );
}

// ---------------------------------------------------------------------------
// struct.sail shapes
// ---------------------------------------------------------------------------

#[test]
fn struct_block_matches_expected_lean() {
    let mut ix = SimpleInterner::new();
    let td = Node::dummy(TypeDefKind::Record {
        name: name(&mut ix, "My_struct"),
        quant: vec![],
        fields: vec![
            (name(&mut ix, "field1"), typ_id(&mut ix, "int")),
            (name(&mut ix, "field2"), typ_id(&mut ix, "bit")),
        ],
        wf: false,
    });
    let (got, printed) = render(&td, &ix);
    assert_eq!(
        got,
        "structure My_struct where\n  field1 : Int\n  field2 : (BitVec 1)\n  deriving BEq, Inhabited, Repr\n"
    );
    assert_eq!(printed.opened, None);
}

#[test]
fn polymorphic_struct_infers_nat_binders() {
    let mut ix = SimpleInterner::new();
    let kn = kid(&mut ix, "'n");
    let kvasize = kid(&mut ix, "'vasize");
    let kpa = kid(&mut ix, "'pa");
    let kts = kid(&mut ix, "'ts");
    let kak = kid(&mut ix, "'arch_ak");
    let gt: Constraint = Node::dummy(ConstraintKind::Gt(nexp_var(kn), nexp_const("0")));
    let ge: Constraint = Node::dummy(ConstraintKind::Ge(nexp_var(kvasize), nexp_const("0")));
    // Sail `'n > 0 & 'vasize >= 0` is a single `And` constraint node.
    let both: Constraint = Node::dummy(ConstraintKind::And(Box::new(gt), Box::new(ge)));
    let bits_vasize = typ_app(&mut ix, "bits", vec![targ_nexp(nexp_var(kvasize))]);
    let opt_bits_vasize = typ_app(&mut ix, "option", vec![targ_typ(bits_vasize)]);
    let eight_times_n = Node::dummy(NExpKind::Times(
        Box::new(nexp_const("8")),
        Box::new(nexp_var(kn)),
    ));
    let bits_8n = typ_app(&mut ix, "bits", vec![targ_nexp(eight_times_n)]);
    let opt_bits_8n = typ_app(&mut ix, "option", vec![targ_typ(bits_8n)]);
    let bool_ty = typ_id(&mut ix, "bool");
    let opt_bool = typ_app(&mut ix, "option", vec![targ_typ(bool_ty)]);
    let int_n = typ_app(&mut ix, "int", vec![targ_nexp(nexp_var(kn))]);
    let td = Node::dummy(TypeDefKind::Record {
        name: name(&mut ix, "My_mem_write_request"),
        quant: vec![
            qid(Kind::Int, kn),
            qid(Kind::Int, kvasize),
            qid(Kind::Type, kpa),
            qid(Kind::Type, kts),
            qid(Kind::Type, kak),
            QuantItem::Constraint(both),
        ],
        fields: vec![
            (name(&mut ix, "va"), opt_bits_vasize),
            (name(&mut ix, "pa"), typ_var(kpa)),
            (name(&mut ix, "translation"), typ_var(kts)),
            (name(&mut ix, "size"), int_n),
            (name(&mut ix, "value"), opt_bits_8n),
            (name(&mut ix, "tag"), opt_bool),
        ],
        wf: false,
    });
    let (got, _) = render(&td, &ix);
    assert_eq!(
        got,
        "/-- Type quantifiers: k_n : Int, k_vasize : Int, k_pa : Type, k_ts : Type, k_arch_ak : Type, k_n > 0 ∧ k_vasize ≥ 0 -/\n\
         structure My_mem_write_request (k_n : Nat) (k_vasize : Nat) (k_pa : Type) (k_ts : Type) (k_arch_ak : Type) where\n\
         \x20 va : (Option (BitVec k_vasize))\n\
         \x20 pa : k_pa\n\
         \x20 translation : k_ts\n\
         \x20 size : Nat\n\
         \x20 value : (Option (BitVec (8 * k_n)))\n\
         \x20 tag : (Option Bool)\n\
         \x20 deriving BEq, Inhabited, Repr\n"
    );
}

// ---------------------------------------------------------------------------
// abbrevs: prelude `bit` / `bits`, nexp/bool/range bodies, fp_bits skip
// ---------------------------------------------------------------------------

#[test]
fn abbrev_bit_and_bits_match_expected_lean() {
    let mut ix = SimpleInterner::new();
    let kn = kid(&mut ix, "'n");
    let bit = Node::dummy(TypeDefKind::Abbrev {
        name: name(&mut ix, "bit"),
        quant: vec![],
        arg: targ_typ(typ_app(
            &mut ix,
            "bitvector",
            vec![targ_nexp(nexp_const("1"))],
        )),
    });
    let bits = Node::dummy(TypeDefKind::Abbrev {
        name: name(&mut ix, "bits"),
        quant: vec![qid(Kind::Int, kn)],
        arg: targ_typ(typ_app(&mut ix, "bitvector", vec![targ_nexp(nexp_var(kn))])),
    });
    let (bit_got, _) = render(&bit, &ix);
    let (bits_got, _) = render(&bits, &ix);
    assert_eq!(bit_got, "abbrev bit := (BitVec 1)\n");
    assert_eq!(bits_got, "abbrev bits k_n := (BitVec k_n)\n");
}

#[test]
fn fp_bits_abbrev_is_skipped_with_comment() {
    let mut ix = SimpleInterner::new();
    let td = Node::dummy(TypeDefKind::Abbrev {
        name: name(&mut ix, "fp_bits"),
        quant: vec![],
        arg: targ_typ(typ_id(&mut ix, "bit")),
    });
    let (got, printed) = render(&td, &ix);
    assert_eq!(got, "-- Abbreviation fp_bits skipped\n");
    assert_eq!(printed.opened, None);
}

#[test]
fn nexp_bool_and_range_abbrevs() {
    let mut ix = SimpleInterner::new();
    let xlen = Node::dummy(TypeDefKind::Abbrev {
        name: name(&mut ix, "xlen"),
        quant: vec![],
        arg: Node::dummy(TypArgKind::NExp(nexp_const("64"))),
    });
    let flag = Node::dummy(TypeDefKind::Abbrev {
        name: name(&mut ix, "flag"),
        quant: vec![],
        arg: Node::dummy(TypArgKind::Bool(Box::new(Node::dummy(
            ConstraintKind::True,
        )))),
    });
    let addr = Node::dummy(TypeDefKind::Abbrev {
        name: name(&mut ix, "addr"),
        quant: vec![],
        arg: targ_typ(typ_app(
            &mut ix,
            "range",
            vec![targ_nexp(nexp_const("0")), targ_nexp(nexp_const("63"))],
        )),
    });
    let (xlen_got, _) = render(&xlen, &ix);
    let (flag_got, _) = render(&flag, &ix);
    let (addr_got, _) = render(&addr, &ix);
    assert_eq!(xlen_got, "abbrev xlen : Int := 64\n");
    assert_eq!(flag_got, "abbrev flag : Bool := true\n");
    assert_eq!(addr_got, "abbrev addr := Nat\n");
}

// ---------------------------------------------------------------------------
// provably_nneg subset: atom/range consult the typedef quantifiers,
// not just constants (pretty_print_lean.ml:298,314,324)
// ---------------------------------------------------------------------------

fn qge(k: Symbol, bound: &str) -> QuantItem {
    QuantItem::Constraint(Node::dummy(ConstraintKind::Ge(
        nexp_var(k),
        nexp_const(bound),
    )))
}

fn qeq(k: Symbol, bound: &str) -> QuantItem {
    QuantItem::Constraint(Node::dummy(ConstraintKind::Equal(
        Box::new(targ_nexp(nexp_var(k))),
        Box::new(targ_nexp(nexp_const(bound))),
    )))
}

fn atom_of(ix: &mut SimpleInterner, k: Symbol) -> Typ {
    typ_app(ix, "atom", vec![targ_nexp(nexp_var(k))])
}

#[test]
fn atom_of_constrained_kid_prints_nat() {
    let mut ix = SimpleInterner::new();
    let kn = kid(&mut ix, "'n");
    let td = Node::dummy(TypeDefKind::Abbrev {
        name: name(&mut ix, "w"),
        quant: vec![qid(Kind::Int, kn), qge(kn, "0")],
        arg: targ_typ(atom_of(&mut ix, kn)),
    });
    let (got, _) = render(&td, &ix);
    assert_eq!(got, "abbrev w k_n := Nat\n");
}

#[test]
fn atom_of_unconstrained_kid_prints_int() {
    let mut ix = SimpleInterner::new();
    let kn = kid(&mut ix, "'n");
    let td = Node::dummy(TypeDefKind::Abbrev {
        name: name(&mut ix, "w"),
        quant: vec![qid(Kind::Int, kn)],
        arg: targ_typ(atom_of(&mut ix, kn)),
    });
    let (got, _) = render(&td, &ix);
    assert_eq!(got, "abbrev w k_n := Int\n");
}

#[test]
fn atom_of_equal_constrained_kid_prints_nat() {
    let mut ix = SimpleInterner::new();
    let kn = kid(&mut ix, "'n");
    let td = Node::dummy(TypeDefKind::Abbrev {
        name: name(&mut ix, "w"),
        quant: vec![qid(Kind::Int, kn), qeq(kn, "8")],
        arg: targ_typ(atom_of(&mut ix, kn)),
    });
    let (got, _) = render(&td, &ix);
    assert_eq!(got, "abbrev w k_n := Nat\n");
}

#[test]
fn range_with_bounded_var_lo_prints_nat() {
    let mut ix = SimpleInterner::new();
    let klo = kid(&mut ix, "'lo");
    let body = typ_app(
        &mut ix,
        "range",
        vec![targ_nexp(nexp_var(klo)), targ_nexp(nexp_const("63"))],
    );
    let td = Node::dummy(TypeDefKind::Abbrev {
        name: name(&mut ix, "r"),
        quant: vec![qid(Kind::Int, klo), qge(klo, "0")],
        arg: targ_typ(body),
    });
    let (got, _) = render(&td, &ix);
    assert_eq!(got, "abbrev r (k_lo : Nat) := Nat\n");
}

#[test]
fn record_field_atom_sees_typedef_quant() {
    let mut ix = SimpleInterner::new();
    let kn = kid(&mut ix, "'n");
    let field_ty = atom_of(&mut ix, kn);
    let td = Node::dummy(TypeDefKind::Record {
        name: name(&mut ix, "S"),
        quant: vec![qid(Kind::Int, kn), qge(kn, "0")],
        fields: vec![(name(&mut ix, "v"), field_ty)],
        wf: false,
    });
    let (got, _) = render(&td, &ix);
    assert!(got.contains("  v : Nat\n"), "{got}");
}

// ---------------------------------------------------------------------------
// bitfield.sail: hard error with span (desugared before printing upstream)
// ---------------------------------------------------------------------------

#[test]
fn bitfield_is_hard_error_with_span() {
    let mut ix = SimpleInterner::new();
    let td_span = Span::new(FileId(3), 10, 40);
    let bv8 = typ_app(&mut ix, "bits", vec![targ_nexp(nexp_const("8"))]);
    let td = TypeDef {
        annotation: None,
        id: NodeId::DUMMY,
        span: td_span,
        kind: TypeDefKind::Bitfield {
            name: name(&mut ix, "cr_type"),
            typ: bv8,
            fields: vec![],
        },
    };
    let err = printer().render_typedef(&td, &ix).unwrap_err();
    match err {
        Error::Unsupported { msg, span } => {
            assert_eq!(span, td_span);
            assert!(msg.contains("cr_type"), "{msg}");
            assert!(msg.contains("TD_bitfield"), "{msg}");
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn abstract_is_hard_error_with_span() {
    let mut ix = SimpleInterner::new();
    let td_span = Span::new(FileId(5), 0, 9);
    let td = TypeDef {
        annotation: None,
        id: NodeId::DUMMY,
        span: td_span,
        kind: TypeDefKind::Abstract {
            name: name(&mut ix, "opaque_t"),
            kind: Kind::Type,
        },
    };
    let err = printer().render_typedef(&td, &ix).unwrap_err();
    match err {
        Error::Unsupported { msg, span } => {
            assert_eq!(span, td_span);
            assert!(msg.contains("TD_abstract"), "{msg}");
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn untranslatable_field_type_is_hard_error() {
    let mut ix = SimpleInterner::new();
    let int_ty = typ_id(&mut ix, "int");
    let bad: Typ = Node::dummy(TypKind::Fn {
        args: vec![],
        ret: Box::new(int_ty),
    });
    let td = Node::dummy(TypeDefKind::Record {
        name: name(&mut ix, "S"),
        quant: vec![],
        fields: vec![(name(&mut ix, "f"), bad)],
        wf: false,
    });
    let err = printer().render_typedef(&td, &ix).unwrap_err();
    assert!(matches!(err, Error::Unsupported { .. }), "{err:?}");
}

#[test]
fn unknown_symbol_is_hard_error() {
    use sail_ast::Symbol;
    let ix = SimpleInterner::new();
    let dangling = Name::new(NodeId::DUMMY, span(), Symbol(9999), false);
    let td = Node::dummy(TypeDefKind::Enum {
        name: dangling,
        members: vec![],
        wf: false,
    });
    let err = printer().render_typedef(&td, &ix).unwrap_err();
    assert!(matches!(err, Error::UnknownSymbol { .. }), "{err:?}");
}

#[test]
fn rendered_typedefs_never_contain_sorry() {
    let mut ix = SimpleInterner::new();
    let option = option_typedef(&mut ix);
    let single = Node::dummy(TypeDefKind::Enum {
        name: name(&mut ix, "E"),
        members: vec![name(&mut ix, "A")],
        wf: false,
    });
    for td in [&option, &single] {
        let (got, _) = render(td, &ix);
        assert!(!got.contains("sorry"), "{got}");
    }
    // Error paths carry no fallback text either.
    let bv = typ_id(&mut ix, "int");
    let bad = Node::dummy(TypeDefKind::Bitfield {
        name: name(&mut ix, "b"),
        typ: bv,
        fields: vec![],
    });
    let err = printer().render_typedef(&bad, &ix).unwrap_err().to_string();
    assert!(!err.contains("sorry"), "{err}");
}
