#![forbid(unsafe_code)]
//! Loop combinators for the pure-Rust Sail compiler (`while#`, `until#`,
//! `foreach#` and the fuel variants `while#t` / `until#t`).
//!
//! Pure-Rust port of `doc_loop` (plus the `E_for` case of `doc_exp`) in
//! `sail/src/sail_lean_backend/pretty_print_lean.ml` (lines ~769–935).
//!
//! ## What is ported
//!
//! * [`LoopKind`]: the five ANF loop combinators plus the surface `for`
//!   expression. Mirrors the OCaml `` `While`` / `` `WhileFuel`` / `` `Until``
//!   / `` `UntilFuel`` polymorphic variants and the `foreach#` / `E_for`
//!   cases, including [`LoopKind::sail_name`] (`"while#"`, …) and
//!   [`is_loop_name`].
//! * [`Order`]: `foreach` step direction; `Dec` negates the step expression
//!   (`minus ^^ step_pp` in OCaml).
//! * [`LoopVars`] / [`LoopCtx`]: `loop_vars`-Verwaltung. `make_loop_vars`
//!   (single variable vs. tuple) and `name_loop_vars` (a `loop_level`
//!   counter producing `loop_vars`, `loop_vars_1`, … so nested loops cannot
//!   collide).
//! * [`effect`]: the `Id.run`-vs-`do` decision after effect analysis:
//!   [`effect::update_arrow`] (`← do` vs `:=`), [`effect::wrap_pure`],
//!   [`effect::wrap_left_arrow`], [`effect::wrap_do`], and
//!   [`effect::function_frame`] (the `SailME.run do` / `do` / `Id.run do` /
//!   `ExceptM.run do` choice at function level).
//! * [`render`]: the `while` / `repeat … until` / `whileFuelM` /
//!   `untilFuelM` / `for … in […:…:…]i` idioms, driven by already
//!   pretty-printed fragments plus effect flags (the crate equivalent of
//!   OCaml's `has_effect` / `has_loop` predicates, which need full compiler
//!   effect info).
//!
//! ## What callers provide
//!
//! This crate works on [`Fragment`]s (already-rendered Lean snippets) and
//! effect booleans. Ordering of tuple variables, `loop_i_lower`/`loop_i_upper`
//! range bindings and the stripping of the compiler-generated
//! `let _ = assert true "loop dummy assert"` wrapper (see `doc_loop`'s
//! `body` match) are the caller's responsibility — the ANF/lowering lane owns
//! them. Fuel bodies keep a real `assert true "loop dummy assert"` statement
//! (cf. `while_print_fuel` in `loop.expected.lean`); only the `E_internal_plet`
//! wrapper around it is stripped upstream.

pub mod combinator;
pub mod effect;
pub mod render;
pub mod vars;

pub use combinator::{FuelKind, LoopKind, Order, apply_step, is_loop_name};
pub use effect::{Frame, for_enter_monad, function_frame, update_arrow, wrap_do, wrap_left_arrow, wrap_pure};
pub use render::{ForeachSpec, FuelSpec, LoopBodySpec, UntilSpec, WhileSpec, ascribed_do, render_foreach, render_for_surface, render_fuel, render_until, render_while};
pub use vars::{LoopCtx, LoopVar, LoopVars};

/// An already pretty-printed Lean snippet (condition, body, …).
///
/// Using strings keeps this crate free of AST/effect dependencies; effect
/// flags on the spec structs carry the `has_effect` information.
pub type Fragment = str;
