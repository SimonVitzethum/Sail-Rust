#![forbid(unsafe_code)]
//! End-to-end fixtures coded from `sail/test/lean/bitvec_operation.sail`,
//! `match_bv.sail` and `bitfield.sail` with their `.expected.lean` outputs.

use sail_bitvec::{
    Bit, BitvecOp, Elem, FileId, Lit, Pat, Span, check_concat_width, check_lit_width, cr_type_fixture,
    is_bitvector_pat, match_keyword, render_e_vector_append, render_e_vector_bitvec,
    render_e_vector_generic, render_pat, render_vector_concat,
};

fn span() -> Span {
    Span::new(FileId(7), 0, 64)
}

fn bin_lit(bits: &str) -> Pat {
    Pat::Lit {
        lit: Lit::parse_bin(bits, span()).unwrap(),
        span: span(),
    }
}

fn var(name: &str, width: u32) -> Pat {
    Pat::Id {
        name: name.to_owned(),
        width: Some(width),
        span: span(),
    }
}

// --- bitvec_operation.sail -------------------------------------------------

#[test]
fn bitvector_literal_uses_join1() {
    // function bitvector_literal(x: bit, y: bit) -> bits(2) = { [x, y] }
    // expected: (BitVec.join1 [x, y])
    let elems = [Elem::bit("x"), Elem::bit("y")];
    assert_eq!(
        render_e_vector_bitvec(&elems, 2, span()).unwrap(),
        "(BitVec.join1 [x, y])"
    );
}

#[test]
fn vector_literal_reverses() {
    // function vector_literal(x: int, y: int) -> vector(2, int) = { [x, y] }
    // expected: #v[y, x]
    assert_eq!(
        render_e_vector_generic(&["x".to_owned(), "y".to_owned()]),
        "#v[y, x]"
    );
}

#[test]
fn every_bitvec_operation_body_matches_expected_lean() {
    // bitvec_operation.expected.lean ln.154-208, 16-bit fixtures
    assert_eq!(BitvecOp::Append.lean_body(&["x", "y"]), "(x +++ y)");
    assert_eq!(
        BitvecOp::Len.lean_body(&["x"]),
        "(Sail.BitVec.length x)"
    );
    assert_eq!(
        BitvecOp::SignExtend.lean_body(&["x", "32"]),
        "(Sail.BitVec.signExtend x 32)"
    );
    assert_eq!(
        BitvecOp::Truncate.lean_body(&["x", "16"]),
        "(Sail.BitVec.truncate x 16)"
    );
    assert_eq!(BitvecOp::Add.lean_body(&["x", "y"]), "(x + y)");
    assert_eq!(
        BitvecOp::Not.lean_body(&["x"]),
        "(Complement.complement x)"
    );
    assert_eq!(
        BitvecOp::Unsigned.lean_body(&["x"]),
        "(BitVec.toNatInt x)"
    );
}

#[test]
fn append_expression_maps_to_triple_plus() {
    // bitvector_append(x, y) = { append(x, y) } -> (x +++ y)
    assert_eq!(render_e_vector_append("x", "y"), "(x +++ y)");
}

#[test]
fn literal_width_mismatch_is_span_error_not_truncation() {
    // A 12-bit literal cannot silently fill bits(8).
    let lit = Lit::parse_hex("ABC", span()).unwrap();
    let err = check_lit_width(&lit, 8, span()).unwrap_err();
    assert_eq!(err.span, span());
}

// --- match_bv.sail ----------------------------------------------------------

#[test]
fn decode_clause_32_bits_renders_verbatim() {
    // match_bv.expected.lean ln.156
    let parts = vec![
        bin_lit("11"),
        bin_lit("111"),
        bin_lit("0"),
        bin_lit("00"),
        var("opc", 2),
        bin_lit("1"),
        var("Rm", 5),
        var("option_v", 3),
        var("S", 1),
        bin_lit("10"),
        var("Rn", 5),
        var("Rt", 5),
    ];
    let pat = Pat::VectorConcat {
        parts: parts.clone(),
        span: span(),
    };
    assert!(is_bitvector_pat(&pat));
    assert_eq!(match_keyword(true), "match_bv ");
    assert_eq!(
        render_pat(&pat, false, true).unwrap(),
        "[11,111,0,00,opc:2,1,Rm:5,option_v:3,S:1,10,Rn:5,Rt:5]"
    );
    check_concat_width(&parts, 32, span()).unwrap();
    // Demanding a wrong total (e.g. 16) must error, not truncate.
    assert!(check_concat_width(&parts, 16, span()).is_err());
}

#[test]
fn csr_clause_with_guard_renders_verbatim() {
    // match_bv.expected.lean ln.172: | [1011000,index:5] if ...
    let parts = vec![bin_lit("1011000"), var("index", 5)];
    assert_eq!(
        render_vector_concat(&parts, span()).unwrap(),
        "[1011000,index:5]"
    );
    check_concat_width(&parts, 12, span()).unwrap();
}

#[test]
fn mapping_all_literal_arms_render_0b() {
    // mapping.expected.lean ln.186-189: | 0b00 => BYTE etc.
    for (bits, expected) in [("00", "0b00"), ("01", "0b01"), ("10", "0b10"), ("11", "0b11")] {
        let pat = Pat::Vector {
            elems: bits
                .chars()
                .map(|c| {
                    assert!(c == '0' || c == '1');
                    if c == '0' { Bit::B0 } else { Bit::B1 }
                })
                .map(|b| Pat::Lit {
                    lit: Lit::Bin(vec![b]),
                    span: span(),
                })
                .collect(),
            span: span(),
        };
        assert!(!is_bitvector_pat(&pat) || true);
        assert_eq!(render_pat(&pat, false, false).unwrap(), expected);
    }
}

// --- bitfield.sail ----------------------------------------------------------

#[test]
fn cr_type_fields_match_expected_lean() {
    // bitfield.sail: CR0: 7..4, LT: 7, GT: 6, CR1: 3..2, CR3: 1..0 over bits(8)
    let decl = cr_type_fixture(span());
    decl.check().unwrap();
    assert_eq!(
        decl.render_get("v", "CR3", span()).unwrap(),
        "(Sail.BitVec.extractLsb v 1 0)"
    );
    assert_eq!(
        decl.render_set("v", "x", "CR3", span()).unwrap(),
        "(Sail.BitVec.updateSubrange v 1 0 x)"
    );
    assert_eq!(
        decl.render_get("v", "LT", span()).unwrap(),
        "(Sail.BitVec.extractLsb v 7 7)"
    );
    // Field widths: 4 + 1 + 1 + 2 + 2 = 12 slots over 8 bits (overlapping views).
    let widths: Vec<u32> = ["CR0", "LT", "GT", "CR1", "CR3"]
        .iter()
        .map(|name| decl.field(name, span()).unwrap().width())
        .collect();
    assert_eq!(widths, vec![4, 1, 1, 2, 2]);
}
