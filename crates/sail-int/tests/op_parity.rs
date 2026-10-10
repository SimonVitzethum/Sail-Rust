#![forbid(unsafe_code)]
//! Lane-127-Paritaet Op-Tabellen (Code, E3: NICHT ausfuehren).
//!
//! Wahrheit: `sail-extern` (`sail_extern::Op`/`op_of_extern`, voll gegen
//! `test/lean/extern*.expected.lean` gepinnt). `sail-int` (`LeanOp`,
//! `Option<&str>`-Signatur) ist die kompatible Sicht; `render_binop`/
//! `render_unop` delegieren seit Lane 127 an `sail-extern`.

use sail_extern::Op;
use sail_int::{op_of_extern, render_binop, render_unop, unop_prefix, LeanOp};

/// Jede `sail-int`-Binop-/Unop-Zeichenkette stimmt mit der kanonischen
/// `sail-extern`-Tabelle ueberein (`None` = kein Extern = `NotOp`).
#[test]
fn op_strings_match_canonical_table() {
    let cases: &[(&str, Op)] = &[
        ("_lean_and", Op::BinOp("&&")),
        ("_lean_or", Op::BinOp("||")),
        ("_lean_beq", Op::BinOp("==")),
        ("_lean_bne", Op::BinOp("!=")),
        ("_lean_not", Op::UnOp("!")),
        ("_lean_add", Op::BinOp("+")),
        ("_lean_addi", Op::BinOp("+i")),
        ("_lean_sub", Op::BinOp("-")),
        ("_lean_subi", Op::BinOp("-i")),
        ("_lean_mul", Op::BinOp("*")),
        ("_lean_muli", Op::BinOp("*i")),
        ("_lean_div", Op::BinOp("/")),
        ("_lean_app", Op::BinOp("+++")),
        ("_lean_bvand", Op::BinOp("&&&")),
        ("_lean_bvor", Op::BinOp("|||")),
        ("_lean_bvxor", Op::BinOp("^^^")),
        ("_lean_shiftl", Op::BinOp("<<<")),
        ("_lean_shiftr", Op::BinOp(">>>")),
        ("_lean_lt", Op::BinOp("<b")),
        ("_lean_ge", Op::BinOp("≥b")),
        ("_lean_le", Op::BinOp("≤b")),
        ("_lean_gt", Op::BinOp(">b")),
        ("_lean_pow2", Op::UnOp("2 ^")),
        ("_lean_pow2i", Op::UnOp("2 ^i")),
    ];
    for (code, want) in cases.iter().copied() {
        assert_eq!(sail_extern::op_of_extern(code), want, "kanonisch {code}");
        match (op_of_extern(Some(code)), want) {
            (LeanOp::Binop(a), Op::BinOp(b)) => assert_eq!(a, b, "{code}"),
            (LeanOp::Unop(a), Op::UnOp(b)) => assert_eq!(a, b, "{code}"),
            (got, w) => panic!("Drift bei {code}: sail-int {got:?} vs sail-extern {w:?}"),
        }
    }
    // `None` (kein Lean-Extern) und direkte Calls sind beidseitig `NotOp`.
    assert_eq!(op_of_extern(None), LeanOp::NotOp);
    assert_eq!(sail_extern::op_of_extern("Int.tdiv"), Op::NotOp);
    assert_eq!(op_of_extern(Some("Int.tdiv")), LeanOp::NotOp);
    // `_lean_pow2i` hat keinen `unop_prefix` (nur `_lean_pow2`).
    assert_eq!(unop_prefix(Some("_lean_pow2")), Some("2 ^ "));
    assert_eq!(unop_prefix(Some("_lean_pow2i")), None);
}

/// Delegation statt Doppelimplementierung: identische Renderings.
#[test]
fn render_delegates_to_canonical_impl() {
    assert_eq!(render_binop("+i", "res", "1"), sail_extern::render_binop("+i", "res", "1"));
    assert_eq!(render_binop("+i", "res", "1"), "(res +i 1)");
    assert_eq!(render_unop("2 ^", "n"), sail_extern::render_unop("2 ^", "n"));
    assert_eq!(render_unop("2 ^", "n"), "(2 ^ n)");
}
