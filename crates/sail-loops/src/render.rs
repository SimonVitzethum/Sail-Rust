#![forbid(unsafe_code)]
//! The `while` / `repeat … until` / fuel / `for` idioms (`doc_loop`).
//!
//! Each renderer takes already pretty-printed fragments plus effect flags
//! and joins the OCaml `separate hardline […]` lines with `\n` (2-space
//! continuation indent, mirroring PPrint's `prefix 2 1`).
//!
//! Reference (`pretty_print_lean.ml`, `doc_loop`):
//!
//! ```ocaml
//! let cond_pp = lambda cond_effects base_lambda cond_pp in
//! let loop_cond = wrap_with_left_arrow cond_effects (prefix 2 1 cond_pp vars_pp) in
//! (* `While *)
//! let loop_head = prefix 2 1 (string "while " ^^ loop_cond) (string "do") in
//! let arrow = if body_effects then leftarrowdo else coloneq in
//! let loop_body_1 = string "let " ^^ vartuple_pp ^^ space ^^ coloneq ^^ space ^^ vars_pp in
//! let loop_body = loop_body_1 ^^ hardline ^^ prefix 2 1 (vars_pp ^^ space ^^ arrow) body_pp in
//! separate hardline [vars_dec_pp; full_loop; wrap_with_pure as_monadic vars_pp]
//! ```

use crate::combinator::FuelKind;
use crate::effect::{update_arrow, wrap_left_arrow, wrap_pure};

/// Indent every line of `text` by two spaces (PPrint `prefix 2 1`).
fn indent(text: &str) -> String {
    text.lines().map(|line| format!("  {line}")).collect::<Vec<_>>().join("\n")
}

/// `(λ <bind> => <cond>)`, with ` do` after the binder when the condition
/// has effects (`lambda` closure inside `doc_loop`).
fn cond_lambda(bind: &str, cond: &str, cond_effects: bool) -> String {
    if cond_effects {
        format!("(λ {bind} => do {cond})")
    } else {
        format!("(λ {bind} => {cond})")
    }
}

/// The applied condition `… loop_vars`, wrapped in `(← …)` when the
/// condition has effects (`loop_cond` in OCaml).
fn loop_cond(bind: &str, cond: &str, cond_effects: bool, vars: &str) -> String {
    let applied = format!("{} {vars}", cond_lambda(bind, cond, cond_effects));
    wrap_left_arrow(cond_effects, &applied)
}

/// The per-iteration state rebound + update:
///
/// ```text
/// let <pat> := <vars>
/// <vars> := <body>        (pure body)
/// <vars> ← do <body>      (effectful body)
/// ```
fn state_update(pat: &str, vars: &str, body: &str, body_effects: bool) -> String {
    format!("let {pat} := {vars}\n{vars} {} {body}", update_arrow(body_effects))
}

/// Shared body of the state-threading loops (`while`, `until`, `foreach#`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopBodySpec {
    /// Fresh state name from [`crate::LoopCtx::fresh_vars`].
    pub vars: String,
    /// `vartuple_pp` from `make_loop_vars` (`res`, `(mult, res)`, `()`).
    pub init: String,
    /// Rebound pattern (`let <pat> := <vars>`).
    pub pat: String,
    /// `λ`-binder for the condition (`res`, `(mult, res)`, `_`).
    pub bind: String,
    /// Already-printed condition.
    pub cond: String,
    /// `has_effect cond`.
    pub cond_effects: bool,
    /// Already-printed body.
    pub body: String,
    /// `has_effect body`.
    pub body_effects: bool,
    /// Whether the loop sits in monadic position (trailing `(pure …)`).
    pub as_monadic: bool,
}

impl LoopBodySpec {
    fn tail(&self) -> String {
        wrap_pure(self.as_monadic, &self.vars)
    }

    fn decl(&self) -> String {
        format!("let mut {} := {}", self.vars, self.init)
    }

    fn cond(&self) -> String {
        loop_cond(&self.bind, &self.cond, self.cond_effects, &self.vars)
    }

    fn update(&self) -> String {
        indent(&state_update(&self.pat, &self.vars, &self.body, self.body_effects))
    }
}

/// `while#` without measure: currently unused alias kept for symmetry.
pub type WhileSpec = LoopBodySpec;
/// `until#` without measure: currently unused alias kept for symmetry.
pub type UntilSpec = LoopBodySpec;

/// Render a `while#` loop (`while_loop` in `loop.expected.lean`):
///
/// ```text
/// let mut loop_vars := res
/// while (λ res => (res <b n)) loop_vars do
///   let res := loop_vars
///   loop_vars := ((res +i 1) : Nat)
/// (pure loop_vars)
/// ```
pub fn render_while(spec: &WhileSpec) -> String {
    let head = format!("while {} do", spec.cond());
    [spec.decl(), format!("{head}\n{}", spec.update()), spec.tail()].join("\n")
}

/// Render an `until#` loop (`repeat … until` idiom; no plain-`until`
/// function exists in `loop.sail`, so this follows the OCaml shape
/// directly):
///
/// ```text
/// let mut loop_vars := i
/// repeat
///   let i := loop_vars
///   loop_vars := (pure (i +i 1))
/// until (λ i => (i <b 100)) loop_vars
/// (pure loop_vars)
/// ```
pub fn render_until(spec: &UntilSpec) -> String {
    let looped = format!("repeat\n{}", spec.update());
    [spec.decl(), looped, format!("until {}", spec.cond()), spec.tail()].join("\n")
}

/// A fuel-bounded loop site (`while#t` / `until#t`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuelSpec {
    /// Fresh state name (`loop_vars`).
    pub vars: String,
    /// Tuple pattern for the `fun` binders (`i`, `(mult, res)`).
    pub pat: String,
    /// Already-printed condition, always monadic
    /// (OCaml uses `doc_exp true`).
    pub cond: String,
    /// Initial state (`doc_exp false ctx varstuple`).
    pub init: String,
    /// Already-printed body, always monadic.
    pub body: String,
    /// Trailing `(pure …)` when in monadic position.
    pub as_monadic: bool,
}

/// Render `whileFuelM` / `untilFuelM` (`while_print_fuel` in
/// `loop.expected.lean`):
///
/// ```text
/// let loop_vars ← whileFuelM (fuel :=100) (fun i => (pure (i <b 100))) i
///   fun i => do
///     assert true "loop dummy assert"
///     (pure (i +i 1))
/// (pure loop_vars)
/// ```
///
/// Note `(fuel :=…)` without a space, as printed by OCaml.
pub fn render_fuel(kind: FuelKind, fuel: &str, spec: &FuelSpec) -> String {
    let head = format!(
        "let {} ← {} (fuel :={fuel}) (fun {} => {}) {}",
        spec.vars,
        kind.combinator(),
        spec.pat,
        spec.cond,
        spec.init
    );
    let step = format!("  fun {} => do\n{}", spec.pat, indent(&indent(&spec.body)));
    [head, step, wrap_pure(spec.as_monadic, &spec.vars)].join("\n")
}

/// A `foreach#` loop site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeachSpec {
    /// Range bindings emitted first
    /// (`let loop_i_lower := m`, … — introduced by lowering, not `doc_loop`).
    pub range_binds: Vec<(String, String)>,
    /// Index variable (`i`, `rr`).
    pub var: String,
    /// Already-printed (and order-applied, see [`crate::apply_step`])
    /// range expressions.
    pub lo: String,
    pub hi: String,
    pub step: String,
    /// Fresh state name.
    pub vars: String,
    /// `vartuple_pp` / rebound pattern.
    pub init: String,
    pub pat: String,
    /// Already-printed body.
    pub body: String,
    /// `has_effect body`.
    pub body_effects: bool,
    /// Trailing `(pure …)` when in monadic position.
    pub as_monadic: bool,
}

/// Render `foreach#` (`foreach_loop` in `loop.expected.lean`):
///
/// ```text
/// let loop_i_lower := m
/// let loop_i_upper := n
/// let mut loop_vars := res
/// for i in [loop_i_lower:loop_i_upper:1]i do
///   let res := loop_vars
///   loop_vars := (res +i 1)
/// (pure loop_vars)
/// ```
pub fn render_foreach(spec: &ForeachSpec) -> String {
    let mut lines: Vec<String> = spec
        .range_binds
        .iter()
        .map(|(name, value)| format!("let {name} := {value}"))
        .collect();
    lines.push(format!("let mut {} := {}", spec.vars, spec.init));
    let head =
        format!("for {} in [{}:{}:{}]i do", spec.var, spec.lo, spec.hi, spec.step);
    lines.push(format!("{head}\n{}", indent(&state_update(&spec.pat, &spec.vars, &spec.body, spec.body_effects))));
    lines.push(wrap_pure(spec.as_monadic, &spec.vars));
    lines.join("\n")
}

/// Render a surface `E_for` (OCaml `E_for` case, e.g. `to_bytes_le`):
///
/// ```text
/// Id.run for i in [loop_i_lower:loop_i_upper:1]i do
///   <body>
/// ```
///
/// The `Id.run` prefix appears exactly when the body is pure
/// (see [`crate::for_enter_monad`]).
pub fn render_for_surface(
    var: &str,
    lo: &str,
    hi: &str,
    step: &str,
    body: &str,
    body_effects: bool,
) -> String {
    let head = format!("for {var} in [{lo}:{hi}:{step}]i do");
    let head = match crate::for_enter_monad(body_effects) {
        Some(enter) => format!("{enter} {head}"),
        None => head,
    };
    format!("{head}\n{}", indent(body))
}

/// Embed a loop as a monad-typed value (`(( do … ) : M T)`).
///
/// This is the `(d_of_arg …)` wrapping for loop arguments in value position,
/// cf. `while_loopmultiplevar` in `loop.expected.lean`:
///
/// ```text
/// let (mult, res) ← (( do
///   <loop>
///   (pure loop_vars) ) : Id (Nat × Nat) )
/// ```
pub fn ascribed_do(pat: &str, monad_ty: &str, inner: &str) -> String {
    let mut lines: Vec<String> =
        indent(inner).lines().map(str::to_owned).collect();
    if let Some(last) = lines.pop() {
        lines.push(format!("{last} ) : {monad_ty} )"));
    }
    let mut out = vec![format!("let {pat} ← (( do")];
    out.extend(lines);
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pure_while() -> WhileSpec {
        LoopBodySpec {
            vars: "loop_vars".to_owned(),
            init: "res".to_owned(),
            pat: "res".to_owned(),
            bind: "res".to_owned(),
            cond: "(res <b n)".to_owned(),
            cond_effects: false,
            body: "((res +i 1) : Nat)".to_owned(),
            body_effects: false,
            as_monadic: true,
        }
    }

    #[test]
    fn while_pure_matches_expected_idiom() {
        assert_eq!(
            render_while(&pure_while()),
            "let mut loop_vars := res\n\
             while (λ res => (res <b n)) loop_vars do\n\
             \x20 let res := loop_vars\n\
             \x20 loop_vars := ((res +i 1) : Nat)\n\
             (pure loop_vars)"
        );
    }

    #[test]
    fn while_monadic_cond_gets_do_and_arrow() {
        // while_loopmon in loop.expected.lean.
        let spec = LoopBodySpec {
            vars: "loop_vars".to_owned(),
            init: "()".to_owned(),
            pat: "()".to_owned(),
            bind: "_".to_owned(),
            cond: "(pure ((← readReg r) <b n))".to_owned(),
            cond_effects: true,
            body: "writeReg r ((← readReg r) +i 1)".to_owned(),
            body_effects: true,
            as_monadic: true,
        };
        assert_eq!(
            render_while(&spec),
            "let mut loop_vars := ()\n\
             while (← (λ _ => do (pure ((← readReg r) <b n))) loop_vars) do\n\
             \x20 let () := loop_vars\n\
             \x20 loop_vars ← do writeReg r ((← readReg r) +i 1)\n\
             (pure loop_vars)"
        );
    }

    #[test]
    fn while_non_monadic_tail_is_bare() {
        let spec = WhileSpec { as_monadic: false, ..pure_while() };
        assert!(render_while(&spec).ends_with("\nloop_vars"));
    }

    #[test]
    fn until_renders_repeat_idiom() {
        let spec = UntilSpec {
            vars: "loop_vars".to_owned(),
            init: "i".to_owned(),
            pat: "i".to_owned(),
            bind: "i".to_owned(),
            cond: "(i <b 100)".to_owned(),
            cond_effects: false,
            body: "(i +i 1)".to_owned(),
            body_effects: false,
            as_monadic: true,
        };
        assert_eq!(
            render_until(&spec),
            "let mut loop_vars := i\n\
             repeat\n\
             \x20 let i := loop_vars\n\
             \x20 loop_vars := (i +i 1)\n\
             until (λ i => (i <b 100)) loop_vars\n\
             (pure loop_vars)"
        );
    }

    #[test]
    fn fuel_spacing_matches_ocaml() {
        // while_print_fuel in loop.expected.lean (assert retained in body).
        let spec = FuelSpec {
            vars: "loop_vars".to_owned(),
            pat: "i".to_owned(),
            cond: "(pure (i <b 100))".to_owned(),
            init: "i".to_owned(),
            body: "assert true \"loop dummy assert\"\n(pure (i +i 1))".to_owned(),
            as_monadic: true,
        };
        assert_eq!(
            render_fuel(FuelKind::WhileFuel, "100", &spec),
            "let loop_vars ← whileFuelM (fuel :=100) (fun i => (pure (i <b 100))) i\n\
             \x20 fun i => do\n\
             \x20   assert true \"loop dummy assert\"\n\
             \x20   (pure (i +i 1))\n\
             (pure loop_vars)"
        );
    }

    #[test]
    fn until_fuel_uses_until_combinator() {
        let spec = FuelSpec {
            vars: "loop_vars".to_owned(),
            pat: "i".to_owned(),
            cond: "(pure (i <b 100))".to_owned(),
            init: "i".to_owned(),
            body: "(pure (i +i 1))".to_owned(),
            as_monadic: true,
        };
        let out = render_fuel(FuelKind::UntilFuel, "n", &spec);
        assert!(out.starts_with("let loop_vars ← untilFuelM (fuel :=n)"), "{out}");
    }

    #[test]
    fn foreach_pure_matches_expected_idiom() {
        // foreach_loop in loop.expected.lean.
        let spec = ForeachSpec {
            range_binds: vec![
                ("loop_i_lower".to_owned(), "m".to_owned()),
                ("loop_i_upper".to_owned(), "n".to_owned()),
            ],
            var: "i".to_owned(),
            lo: "loop_i_lower".to_owned(),
            hi: "loop_i_upper".to_owned(),
            step: "1".to_owned(),
            vars: "loop_vars".to_owned(),
            init: "res".to_owned(),
            pat: "res".to_owned(),
            body: "(res +i 1)".to_owned(),
            body_effects: false,
            as_monadic: true,
        };
        assert_eq!(
            render_foreach(&spec),
            "let loop_i_lower := m\n\
             let loop_i_upper := n\n\
             let mut loop_vars := res\n\
             for i in [loop_i_lower:loop_i_upper:1]i do\n\
             \x20 let res := loop_vars\n\
             \x20 loop_vars := (res +i 1)\n\
             (pure loop_vars)"
        );
    }

    #[test]
    fn foreach_effectful_body_uses_update_arrow() {
        // foreach_loopmon in loop.expected.lean.
        let spec = ForeachSpec {
            range_binds: vec![
                ("loop_i_lower".to_owned(), "n".to_owned()),
                ("loop_i_upper".to_owned(), "m".to_owned()),
            ],
            var: "i".to_owned(),
            lo: "loop_i_lower".to_owned(),
            hi: "loop_i_upper".to_owned(),
            step: "1".to_owned(),
            vars: "loop_vars".to_owned(),
            init: "()".to_owned(),
            pat: "()".to_owned(),
            body: "writeReg r ((← readReg r) +i 1)".to_owned(),
            body_effects: true,
            as_monadic: true,
        };
        let out = render_foreach(&spec);
        assert!(out.contains("loop_vars ← do writeReg r ((← readReg r) +i 1)"), "{out}");
        assert!(out.contains("let () := loop_vars"), "{out}");
    }

    #[test]
    fn surface_for_pure_body_gets_id_run() {
        // Inner loop of foreachpure in loop.expected.lean.
        assert_eq!(
            render_for_surface("i", "0", "(m -i 1)", "1", "(nothing ())", false),
            "Id.run for i in [0:(m -i 1):1]i do\n  (nothing ())"
        );
    }

    #[test]
    fn surface_for_effectful_body_has_no_id_run() {
        // Inner loop of foreachnotsopure in loop.expected.lean.
        assert_eq!(
            render_for_surface("i", "0", "(m -i 1)", "1", "writeReg r x", true),
            "for i in [0:(m -i 1):1]i do\n  writeReg r x"
        );
    }

    #[test]
    fn ascribed_do_matches_multivar_idiom() {
        let inner = render_while(&LoopBodySpec {
            vars: "loop_vars".to_owned(),
            init: "(mult, res)".to_owned(),
            pat: "(mult, res)".to_owned(),
            bind: "(mult, res)".to_owned(),
            cond: "(res <b n)".to_owned(),
            cond_effects: false,
            body: "(pure res)".to_owned(),
            body_effects: false,
            as_monadic: true,
        });
        assert_eq!(
            ascribed_do("(mult, res)", "Id (Nat × Nat)", &inner),
            "let (mult, res) ← (( do\n\
             \x20 let mut loop_vars := (mult, res)\n\
             \x20 while (λ (mult, res) => (res <b n)) loop_vars do\n\
             \x20   let (mult, res) := loop_vars\n\
             \x20   loop_vars := (pure res)\n\
             \x20 (pure loop_vars) ) : Id (Nat × Nat) )"
        );
    }
}
