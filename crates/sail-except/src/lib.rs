#![forbid(unsafe_code)]
//! Early-return / exception mapping for the pure-Rust Sail compiler.
//!
//! Pure-Rust port of the `early_return` / `E_exit` / `E_throw` / `E_try` /
//! `E_assert` cases of `doc_exp` plus the function-frame choice in
//! `sail/src/sail_lean_backend/pretty_print_lean.ml` (lines ~935–937,
//! ~1059–1074, ~1160–1176) and `Rewriter.has_early_return` in
//! `sail/src/lib/rewriter.ml`.
//!
//! ## What is ported
//!
//! * [`detect`]: [`detect::has_early_return`] over a minimal [`detect::Exp`]
//!   tree. OCaml folds with `e_app: id = "early_return" || any arg` and
//!   `e_return _ = true`; `E_throw` / `E_exit` do **not** count.
//! * [`monad`]: the signature return-type rule (`is_monadic → SailM Ret`),
//!   the `(is_monadic, early_return, has_loop)` frame triple
//!   (`SailME.run do` / `do` / `Id.run do` / `ExceptM.run do`), and
//!   `prepend_monad` for `let`-ascriptions
//!   (`SailME E E`, `ExceptM Bool Nat`, …).
//! * [`render`]: `early_return x` → `SailME.throw x` in `SailM`, else
//!   `throw x`; `E_exit → throw Error.Exit` (bare in every position — the
//!   OCaml case applies neither `wrap_with_pure` nor an arrow);
//!   `E_throw → sailThrow (…)` with a `←` prefix only in non-monadic
//!   position; `E_try → sailTryCatch` vs `sailTryCatchE` (chosen by
//!   `has_early_return` of the tried expression); `E_assert → assert c msg`.
//!
//! ## What callers provide
//!
//! This crate works on [`Fragment`]s (already-rendered Lean snippets) and
//! effect booleans, mirroring the sibling-crate convention: full Sail
//! effect analysis (`effectful (effect_of e)`, `Effects.function_is_pure`)
//! and ANF lowering stay upstream. Arrow discipline ("keine falschen `←`")
//! is encoded in the render signatures: [`render::render_throw`] and
//! [`render::render_try_catch`] take `as_monadic`, and the `←` they emit in
//! non-monadic position is concatenated directly (`^^` in OCaml — no space,
//! no parens), exactly as the reference backend does.

pub mod detect;
pub mod monad;
pub mod render;

pub use detect::{has_early_return, Exp};
pub use monad::{ascribed_monad, function_frame, return_type, Frame};
pub use render::{
    early_return_throw, render_assert, render_early_return, render_exit, render_throw,
    render_try_catch, try_catch_combinator,
};

/// An already pretty-printed Lean snippet (argument, body, match arm, …).
///
/// Using strings keeps this crate free of AST/effect dependencies; effect
/// flags on the render functions carry the `has_effect` / `as_monadic`
/// information instead.
pub type Fragment = str;
