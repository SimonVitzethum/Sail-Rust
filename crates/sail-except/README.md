# sail-except

Early-return/exception mapping for the pure-Rust Sail compiler (Lane 28).

Ports the `early_return` / `E_exit` / `E_throw` / `E_try` / `E_assert` cases
of `doc_exp`, the `(is_monadic, early_return, has_loop)` function frame and
`prepend_monad` from `sail/src/sail_lean_backend/pretty_print_lean.ml`, plus
`Rewriter.has_early_return` from `sail/src/lib/rewriter.ml`.

Pure Rust, `#![forbid(unsafe_code)]`, no dependencies. Works on
already-rendered Lean fragments plus effect booleans; full effect analysis
stays upstream.
