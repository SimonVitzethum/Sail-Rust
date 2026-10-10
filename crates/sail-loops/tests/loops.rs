//! Fixture tests built from `sail/test/lean/loop.sail` structures.
//!
//! Each fixture composes a full `def … := <frame>` body through the public
//! API (`LoopCtx` for fresh `loop_vars`, [`sail_loops::Order`] applied via
//! [`sail_loops::apply_step`], [`sail_loops::function_frame`] for the
//! `Id.run do` / `do` choice) and checks the Lean idioms asserted by
//! `loop.expected.lean`.

use sail_loops::{
    ForeachSpec, Frame, FuelKind, FuelSpec, LoopBodySpec, LoopCtx, LoopVar, LoopVars, Order,
    apply_step, for_enter_monad, function_frame, render_foreach, render_for_surface, render_fuel,
    render_while,
};

/// `function foreach_loop(m : nat, n : nat) -> nat` — pure foreach.
#[test]
fn fixture_foreach_loop() {
    assert_eq!(function_frame(false, false, true), Frame::IdRun);
    let mut ctx = LoopCtx::new();
    let vars = ctx.fresh_vars();
    let state = LoopVars::Single(LoopVar::id("res"));
    let out = render_foreach(&ForeachSpec {
        range_binds: vec![
            ("loop_i_lower".to_owned(), "m".to_owned()),
            ("loop_i_upper".to_owned(), "n".to_owned()),
        ],
        var: "i".to_owned(),
        lo: "loop_i_lower".to_owned(),
        hi: "loop_i_upper".to_owned(),
        step: apply_step(Order::Inc, "1"),
        vars,
        init: state.init_expr(),
        pat: state.match_pattern(),
        body: "(res +i 1)".to_owned(),
        body_effects: false,
        as_monadic: true,
    });
    let frame = Frame::IdRun.suffix();
    let def = format!("def foreach_loop (m : Nat) (n : Nat) : Nat := {frame}\n{out}");
    for line in [
        "def foreach_loop (m : Nat) (n : Nat) : Nat := Id.run do",
        "let mut loop_vars := res",
        "for i in [loop_i_lower:loop_i_upper:1]i do",
        "loop_vars := (res +i 1)",
        "(pure loop_vars)",
    ] {
        assert!(def.contains(line), "missing {line:?} in:\n{def}");
    }
}

/// `function while_loopboth` — pure condition, effectful body.
#[test]
fn fixture_while_loopboth() {
    assert_eq!(function_frame(true, false, true), Frame::SailM);
    let mut ctx = LoopCtx::new();
    let out = render_while(&LoopBodySpec {
        vars: ctx.fresh_vars(),
        init: "res".to_owned(),
        pat: "res".to_owned(),
        bind: "res".to_owned(),
        cond: "(res <b n)".to_owned(),
        cond_effects: false,
        body: "writeReg r ((← readReg r) +i res)".to_owned(),
        body_effects: true,
        as_monadic: true,
    });
    assert!(out.contains("while (λ res => (res <b n)) loop_vars do"), "{out}");
    assert!(out.contains("loop_vars ← do writeReg"), "{out}");
}

/// `function while_print_fuel` — fuel variant with retained assert.
#[test]
fn fixture_while_print_fuel() {
    let out = render_fuel(
        FuelKind::WhileFuel,
        "100",
        &FuelSpec {
            vars: "loop_vars".to_owned(),
            pat: "i".to_owned(),
            cond: "(pure (i <b 100))".to_owned(),
            init: "i".to_owned(),
            body: "assert true \"loop dummy assert\"\n(pure (i +i 1))".to_owned(),
            as_monadic: true,
        },
    );
    assert!(out.contains("whileFuelM (fuel :=100)"), "{out}");
    assert!(out.contains("fun i => do"), "{out}");
    assert!(out.contains("assert true \"loop dummy assert\""), "{out}");
}

/// `foreachpure` inner loop: pure body → `Id.run for`.
/// `foreachnotsopure` inner loop: effectful body → bare `for`.
#[test]
fn fixture_nested_foreach_id_run_decision() {
    let mut ctx = LoopCtx::new();
    let _outer = ctx.fresh_vars();
    let _inner = ctx.fresh_vars();
    assert_eq!(ctx.level(), 2);

    let pure_inner =
        render_for_surface("i", "0", "(m -i 1)", "1", "(nothing ())", false);
    assert!(pure_inner.starts_with("Id.run for i in [0:(m -i 1):1]i do"), "{pure_inner}");

    let eff_inner = render_for_surface("i", "0", "(m -i 1)", "1", "writeReg r x", true);
    assert!(eff_inner.starts_with("for i in [0:(m -i 1):1]i do"), "{eff_inner}");
    assert_eq!(for_enter_monad(false), Some("Id.run"));
    assert_eq!(for_enter_monad(true), None);
}

/// `foreach_loopmultiplevar`: tuple state renders `(mult, res)` everywhere.
#[test]
fn fixture_foreach_multivar_tuple_order() {
    let state = LoopVars::Tuple(vec![LoopVar::id("mult"), LoopVar::id("res")]);
    let out = render_foreach(&ForeachSpec {
        range_binds: vec![],
        var: "i".to_owned(),
        lo: "m".to_owned(),
        hi: "n".to_owned(),
        step: apply_step(Order::Inc, "1"),
        vars: "loop_vars".to_owned(),
        init: state.init_expr(),
        pat: state.match_pattern(),
        body: "(mult, res)".to_owned(),
        body_effects: false,
        as_monadic: true,
    });
    assert!(out.contains("let mut loop_vars := (mult, res)"), "{out}");
    assert!(out.contains("let (mult, res) := loop_vars"), "{out}");
}

/// `default Order dec` negates an explicit step.
#[test]
fn fixture_dec_order_negates_step() {
    assert_eq!(apply_step(Order::Dec, "1"), "-1");
    assert_eq!(apply_step(Order::Inc, "1"), "1");
}
