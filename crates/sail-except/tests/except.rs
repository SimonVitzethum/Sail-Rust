#![forbid(unsafe_code)]
//! Fixture tests built from `early_return.sail` / `errors.sail` structures.
//!
//! Each fixture composes a full `def … := <frame>` header through the public
//! API ([`sail_except::return_type`] + [`sail_except::function_frame`]) and
//! checks the exact Lean idioms pinned by the reference outputs
//! `sail/test/lean/early_return.expected.lean`,
//! `sail/test/lean/errors.expected.lean` and
//! `sail/test/lean/try_catch.expected.lean` (differential tests at the
//! bottom assert byte-equality with lines copied verbatim from those files).

use sail_except::{
    ascribed_monad, function_frame, has_early_return, render_assert, render_early_return,
    render_exit, render_throw, render_try_catch, return_type, try_catch_combinator, Exp,
};

const EARLY_RETURN_FIXTURE: &str = include_str!("fixtures/early_return_mini.sail");
const ERRORS_FIXTURE: &str = include_str!("fixtures/errors_mini.sail");

fn def_header(name: &str, args: &str, is_monadic: bool, ret: &str, frame: &str) -> String {
    format!(
        "def {name} ({args}) : {} := {frame}",
        return_type(is_monadic, ret)
    )
}

/// `foreach_earlyreturneffect`: stateful + early return → `SailM`/`SailME.run`.
#[test]
fn fixture_foreach_earlyreturneffect() {
    assert!(EARLY_RETURN_FIXTURE.contains("function foreach_earlyreturneffect"));
    assert!(EARLY_RETURN_FIXTURE.contains("return false"));
    // `return false` inside the loop body counts …
    let body = Exp::app(
        "foreach#",
        vec![Exp::app(
            "if#",
            vec![Exp::EarlyReturn(Box::new(Exp::Leaf)), Exp::Leaf],
        )],
    );
    assert!(has_early_return(&body));
    assert_eq!(
        function_frame(true, true, true),
        sail_except::Frame::SailMeRun
    );

    let header = def_header(
        "foreach_earlyreturneffect",
        "n : Nat",
        true,
        "Bool",
        function_frame(true, true, true).suffix(),
    );
    assert_eq!(
        header,
        "def foreach_earlyreturneffect (n : Nat) : SailM Bool := SailME.run do"
    );
    assert_eq!(
        render_early_return(true, "(false : Bool)"),
        "SailME.throw (false : Bool)"
    );
}

/// `foreach_earlyreturnpure`: pure + early return → plain `Bool`/`ExceptM.run`.
#[test]
fn fixture_foreach_earlyreturnpure() {
    assert!(!ERRORS_FIXTURE.is_empty());
    assert!(EARLY_RETURN_FIXTURE.contains("function foreach_earlyreturnpure"));
    assert_eq!(
        function_frame(false, true, true),
        sail_except::Frame::ExceptMRun
    );

    let header = def_header(
        "foreach_earlyreturnpure",
        "n : Nat",
        false,
        "Bool",
        function_frame(false, true, true).suffix(),
    );
    assert_eq!(
        header,
        "def foreach_earlyreturnpure (n : Nat) : Bool := ExceptM.run do"
    );
    assert_eq!(
        render_early_return(false, "(false : Bool)"),
        "throw (false : Bool)"
    );
    // Loop-header ascription `… : ExceptM Bool Nat`.
    assert_eq!(
        ascribed_monad(false, Some("Bool"), "Nat"),
        "ExceptM Bool Nat"
    );
}

/// `ite_early_return_exit`: `SailME.throw` for the return, bare
/// `throw Error.Exit` for the exit, `SailME E E` ascription for the `let`.
#[test]
fn fixture_ite_early_return_exit() {
    assert!(EARLY_RETURN_FIXTURE.contains("function ite_early_return_exit"));
    assert!(EARLY_RETURN_FIXTURE.contains("exit()"));
    // `exit()` alone is not an early return …
    assert!(!has_early_return(&Exp::Exit));
    // … but the `return r_A` in the same body selects the SailME frame.
    let body = Exp::Seq(vec![Exp::EarlyReturn(Box::new(Exp::Leaf)), Exp::Exit]);
    assert!(has_early_return(&body));

    let header = def_header(
        "ite_early_return_exit",
        "x : Bool",
        true,
        "E",
        function_frame(true, true, false).suffix(),
    );
    assert_eq!(
        header,
        "def ite_early_return_exit (x : Bool) : SailM E := SailME.run do"
    );
    assert_eq!(render_exit(), "throw Error.Exit");
    assert_eq!(ascribed_monad(true, Some("E"), "E"), "SailME E E");
}

/// `errors.sail`: `test_exit` / `test_assert` — plain `SailM` frame, no `←`.
#[test]
fn fixture_errors_exit_and_assert() {
    assert!(ERRORS_FIXTURE.contains("function test_exit"));
    assert!(ERRORS_FIXTURE.contains("exit()"));
    assert!(ERRORS_FIXTURE.contains("assert(b, \"b is false\")"));
    // Neither function contains an early return → plain `do` frame.
    assert_eq!(
        function_frame(true, false, false),
        sail_except::Frame::SailM
    );

    let exit_header = def_header(
        "test_exit",
        "b : Bool",
        true,
        "Unit",
        function_frame(true, false, false).suffix(),
    );
    assert_eq!(exit_header, "def test_exit (b : Bool) : SailM Unit := do");
    assert_eq!(render_exit(), "throw Error.Exit");

    let assert_header = def_header(
        "test_assert",
        "b : Bool",
        true,
        "(BitVec 1)",
        function_frame(true, false, false).suffix(),
    );
    assert_eq!(
        assert_header,
        "def test_assert (b : Bool) : SailM (BitVec 1) := do"
    );
    assert_eq!(
        render_assert("b", "\"b is false\""),
        "assert b \"b is false\""
    );
}

/// `try_catch`: `sailThrow` payload + `sailTryCatch` with `the_exception`.
#[test]
fn fixture_try_catch_prop() {
    // Tried expression throws but has no early return → plain combinator.
    let tried = Exp::Throw(Box::new(Exp::Leaf));
    assert!(!has_early_return(&tried));
    assert_eq!(try_catch_combinator(false), "sailTryCatch");

    let payload = render_throw(true, "(E_bool true)");
    assert_eq!(payload, "sailThrow ((E_bool true))");
    let out = render_try_catch(
        true,
        false,
        &payload,
        "match the_exception with\n| E_bool b => (pure b)\n| E_unit () => (pure false)",
    );
    assert_eq!(
        out,
        "sailTryCatch (sailThrow ((E_bool true))) (fun the_exception => \nmatch the_exception with\n| E_bool b => (pure b)\n| E_unit () => (pure false))"
    );
}

/// A tried block *with* an early return selects `sailTryCatchE`.
#[test]
fn fixture_try_catch_early_return_selects_e() {
    let tried = Exp::Seq(vec![Exp::EarlyReturn(Box::new(Exp::Leaf))]);
    assert!(has_early_return(&tried));
    assert_eq!(try_catch_combinator(true), "sailTryCatchE");
    let out = render_try_catch(true, true, "tried", "arms");
    assert!(out.starts_with("sailTryCatchE (tried)"), "{out}");
    // Non-monadic position adds the bare `←` prefix, never `(← …)`.
    let out_pure = render_try_catch(false, true, "tried", "arms");
    assert!(out_pure.starts_with("←sailTryCatchE (tried)"), "{out_pure}");
}

// ---------------------------------------------------------------------------
// Differential tests: byte-equality against the reference `.expected.lean`
// outputs (pins copied verbatim; file + line cited above each block).
// ---------------------------------------------------------------------------

/// `sail/test/lean/early_return.expected.lean`
/// ll.178, 184–187 (effect) and ll.192, 200–205 (pure).
#[test]
fn differential_early_return_matches_expected_lean() {
    const EFFECT_HEADER: &str =
        "def foreach_earlyreturneffect (n : Nat) : SailM Bool := SailME.run do";
    const EFFECT_THROW: &str = "      then SailME.throw (false : Bool)";
    const PURE_HEADER: &str = "def foreach_earlyreturnpure (n : Nat) : Bool := ExceptM.run do";
    const PURE_THROW: &str = "        then throw (false : Bool)";
    const PURE_ASCRIPTION: &str = "    (pure loop_vars) ) : ExceptM Bool Nat )";

    let effect_header = def_header(
        "foreach_earlyreturneffect",
        "n : Nat",
        true,
        "Bool",
        function_frame(true, true, true).suffix(),
    );
    assert_eq!(effect_header, EFFECT_HEADER);
    let effect_throw = format!("      then {}", render_early_return(true, "(false : Bool)"));
    assert_eq!(effect_throw, EFFECT_THROW);

    let pure_header = def_header(
        "foreach_earlyreturnpure",
        "n : Nat",
        false,
        "Bool",
        function_frame(false, true, true).suffix(),
    );
    assert_eq!(pure_header, PURE_HEADER);
    let pure_throw = format!(
        "        then {}",
        render_early_return(false, "(false : Bool)")
    );
    assert_eq!(pure_throw, PURE_THROW);
    let ascription = format!(
        "    (pure loop_vars) ) : {} )",
        ascribed_monad(false, Some("Bool"), "Nat")
    );
    assert_eq!(ascription, PURE_ASCRIPTION);
}

/// `sail/test/lean/early_return.expected.lean` ll.522–533
/// (`ite_early_return_exit`: ascription + exit under `if`).
#[test]
fn differential_ite_exit_matches_expected_lean() {
    const HEADER: &str = "def ite_early_return_exit (x : Bool) : SailM E := SailME.run do";
    const ASC_LINE: &str = "    else readReg r_B ) : SailME E E )";
    const EXIT_LINE: &str = "  then throw Error.Exit";

    let header = def_header(
        "ite_early_return_exit",
        "x : Bool",
        true,
        "E",
        function_frame(true, true, false).suffix(),
    );
    assert_eq!(header, HEADER);
    let asc = format!(
        "    else readReg r_B ) : {} )",
        ascribed_monad(true, Some("E"), "E")
    );
    assert_eq!(asc, ASC_LINE);
    let exit = format!("  then {}", render_exit());
    assert_eq!(exit, EXIT_LINE);
}

/// `sail/test/lean/errors.expected.lean` ll.162–170.
#[test]
fn differential_errors_matches_expected_lean() {
    const EXIT_DEF: &str = "def test_exit (b : Bool) : SailM Unit := do\n  if (b : Bool)\n  then throw Error.Exit\n  else (pure ())";
    const ASSERT_DEF: &str =
        "def test_assert (b : Bool) : SailM (BitVec 1) := do\n  assert b \"b is false\"\n  (pure 1#1)";

    let exit_def = format!(
        "{}\n  if (b : Bool)\n  then {}\n  else (pure ())",
        def_header(
            "test_exit",
            "b : Bool",
            true,
            "Unit",
            function_frame(true, false, false).suffix()
        ),
        render_exit()
    );
    assert_eq!(exit_def, EXIT_DEF);

    let assert_def = format!(
        "{}\n  {}\n  (pure 1#1)",
        def_header(
            "test_assert",
            "b : Bool",
            true,
            "(BitVec 1)",
            function_frame(true, false, false).suffix()
        ),
        render_assert("b", "\"b is false\"")
    );
    assert_eq!(assert_def, ASSERT_DEF);
}

/// `sail/test/lean/try_catch.expected.lean` ll.15–19.
#[test]
fn differential_try_catch_matches_expected_lean() {
    const PROP_DEF: &str = "def prop (_ : Unit) : SailM Bool := do\n  sailTryCatch (sailThrow ((E_bool true))) (fun the_exception => \n  match the_exception with\n  | E_bool b => (pure b)\n  | E_unit () => (pure false))";

    let tried = render_throw(true, "(E_bool true)");
    let tc = render_try_catch(
        true,
        false,
        &tried,
        "  match the_exception with\n  | E_bool b => (pure b)\n  | E_unit () => (pure false)",
    );
    // The reference wraps the handler in `parens (…)`; render_try_catch
    // already emits that closing paren.
    let prop_def = format!(
        "{}\n  {tc}",
        def_header(
            "prop",
            "_ : Unit",
            true,
            "Bool",
            function_frame(true, false, false).suffix()
        ),
    );
    assert_eq!(prop_def, PROP_DEF);
}
