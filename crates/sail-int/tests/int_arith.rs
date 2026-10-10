#![forbid(unsafe_code)]
//! Fixture tests transcribed from `sail/test/lean/range.sail`,
//! `sail/test/lean/range.expected.lean`, `sail/test/lean/loop.sail`,
//! and `sail/test/lean/loop.expected.lean` (read-only references outside
//! this worktree).

use sail_int::{op_of_extern, render_binop, Facts, ForLoop, LeanOp, NExp, NumType, Order};

fn range(lo: NExp, hi: NExp) -> NumType {
    NumType::Range { lo, hi }
}

// --- range.sail argument types -------------------------------------------
// f_int    : range(0, 31)   -> ...   ; expected `(x : Nat)`
// f_nat    : range(0, 31)   -> ...   ; expected `(x : Nat)`
// f_negvar : range(0, 'n)   -> ...   ; expected `(x : Nat)`
// f_nnegvar: range(0, 'n)   -> ...   ; expected `(x : Nat)`
// f_unkn   : range('n, 'm)  -> ...   ; expected `(x : Int)`

#[test]
fn range_arg_types_match_expected_lean() {
    let r031 = range(NExp::Constant(0), NExp::Constant(31));
    assert_eq!(r031.lean_name(), "Nat"); // f_int, f_nat args
    let r0n = range(NExp::Constant(0), NExp::var("n"));
    assert_eq!(r0n.lean_name(), "Nat"); // f_negvar, f_nnegvar args
    let rnm = range(NExp::var("n"), NExp::var("m"));
    assert_eq!(rnm.lean_name(), "Int"); // f_unkn arg
}

// --- range.sail return types ----------------------------------------------
// f_int    : ... -> range(-2, 2)   ; expected `: Int`
// f_nat    : ... -> range(0, 2)    ; expected `: Nat`
// f_negvar : ... -> range(-'n, 'n) ; expected `: Int`
// f_nnegvar: ... -> range(0, 'n)   ; expected `: Nat`
// f_unkn   : ... -> range('n, 'm)  ; expected `: Int`

#[test]
fn range_return_types_match_expected_lean() {
    assert_eq!(range(NExp::Constant(-2), NExp::Constant(2)).lean_name(), "Int");
    assert_eq!(range(NExp::Constant(0), NExp::Constant(2)).lean_name(), "Nat");
    assert_eq!(
        range(NExp::neg(NExp::var("n")), NExp::var("n")).lean_name(),
        "Int"
    );
    assert_eq!(
        range(NExp::Constant(0), NExp::var("n")).lean_name(),
        "Nat"
    );
    assert_eq!(
        range(NExp::var("n"), NExp::var("m")).lean_name(),
        "Int"
    );
}

// Full `def` signatures from range.expected.lean:
//   def f_int (x : Nat) : Int
//   def f_nat (x : Nat) : Nat
//   def f_negvar (x : Nat) : Int
//   def f_nnegvar (x : Nat) : Nat
//   def f_unkn (x : Int) : Int

#[test]
fn range_def_signatures() {
    let cases = [
        ("f_int", "Nat", "Int"),
        ("f_nat", "Nat", "Nat"),
        ("f_negvar", "Nat", "Int"),
        ("f_nnegvar", "Nat", "Nat"),
        ("f_unkn", "Int", "Int"),
    ];
    let args = [
        range(NExp::Constant(0), NExp::Constant(31)),
        range(NExp::Constant(0), NExp::Constant(31)),
        range(NExp::Constant(0), NExp::var("n")),
        range(NExp::Constant(0), NExp::var("n")),
        range(NExp::var("n"), NExp::var("m")),
    ];
    let rets = [
        range(NExp::Constant(-2), NExp::Constant(2)),
        range(NExp::Constant(0), NExp::Constant(2)),
        range(NExp::neg(NExp::var("n")), NExp::var("n")),
        range(NExp::Constant(0), NExp::var("n")),
        range(NExp::var("n"), NExp::var("m")),
    ];
    for (i, (name, arg, ret)) in cases.iter().enumerate() {
        let sig = format!(
            "def {name} (x : {}) : {}",
            args[i].lean_name(),
            rets[i].lean_name()
        );
        assert_eq!(sig, format!("def {name} (x : {arg}) : {ret}"), "{name}");
    }
}

// A `Nat`-bound variable refines later lookups: with `n >= 0` in context,
// `atom('n)` is `Nat` (cf. `f_nnegvar` body returning `x : Nat`).
#[test]
fn nat_binder_refines_atom() {
    let facts = Facts::new().with_lower("n", 0);
    assert_eq!(
        NumType::Atom(NExp::var("n")).lean_name_in(&facts),
        "Nat"
    );
}

// --- loop.sail integer operator shapes ------------------------------------
// loop.expected.lean uses `(res +i 1)`, `(m -i 1)`, `(res *i mult)`,
// `(x +i 1)`, `(8 *i i)`, `(res +i i)` throughout the Nat/int loops.

#[test]
fn loop_bodies_use_int_ops() {
    assert_eq!(op_of_extern(Some("_lean_addi")), LeanOp::Binop("+i"));
    assert_eq!(render_binop("+i", "res", "1"), "(res +i 1)");
    assert_eq!(op_of_extern(Some("_lean_subi")), LeanOp::Binop("-i"));
    assert_eq!(render_binop("-i", "m", "1"), "(m -i 1)");
    assert_eq!(op_of_extern(Some("_lean_muli")), LeanOp::Binop("*i"));
    assert_eq!(render_binop("*i", "res", "mult"), "(res *i mult)");
    assert_eq!(render_binop("+i", "x", "1"), "(x +i 1)");
    assert_eq!(render_binop("*i", "8", "i"), "(8 *i i)");
    assert_eq!(render_binop("+i", "res", "i"), "(res +i i)");
}

// --- loop.sail E_for heads -------------------------------------------------
// `foreachpure`: pure body, `Ord_inc`, bounds `0 .. (m -i 1) by 1`:
//   `Id.run for i in [0:(m -i 1):1]i do (nothing ())`
// `foreachnotsopure`: effectful body -> no `Id.run` prefix.

#[test]
fn foreachpure_inner_head() {
    let l = ForLoop {
        var: "i".to_string(),
        from: "0".to_string(),
        to: "(m -i 1)".to_string(),
        step: "1".to_string(),
        order: Order::Inc,
        body_effect: false,
    };
    assert_eq!(l.bracket(), "[0:(m -i 1):1]i");
    assert_eq!(l.head(), "Id.run for i in [0:(m -i 1):1]i do");
    assert_eq!(l.order.combinator(), "foreach_Z_up");
}

#[test]
fn foreachnotsopure_head_is_monadic() {
    let l = ForLoop {
        var: "i".to_string(),
        from: "0".to_string(),
        to: "(m -i 1)".to_string(),
        step: "1".to_string(),
        order: Order::Inc,
        body_effect: true,
    };
    assert_eq!(l.head(), "for i in [0:(m -i 1):1]i do");
}

#[test]
fn dec_loop_negates_step() {
    let l = ForLoop {
        var: "rr".to_string(),
        from: "loop_rr_lower".to_string(),
        to: "loop_rr_upper".to_string(),
        step: "1".to_string(),
        order: Order::Dec,
        body_effect: true,
    };
    assert_eq!(l.order.combinator(), "foreach_Z_down");
    assert_eq!(
        l.head(),
        "for rr in [loop_rr_lower:loop_rr_upper:-1]i do"
    );
}
