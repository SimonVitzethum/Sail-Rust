# Canonical, disk-backed typechecking

The ordinary CLI now runs `sail_ty::program::Program` over the canonical
`sail_ast` definitions. It does not translate bodies into the older miniature
checker AST. The existing miniature checker remains for its current users;
its rule catalogue does not imply coverage by this canonical checker.

The declaration pass reads the flat AST store in source order and retains
signatures, constructor kinds and register types. Binding histories preserve
source-position lookup: a later replacement `val` cannot change an earlier
function's signature. These tables are frozen and shared by the body workers;
the whole environment is not copied for each function.

The body pass streams content IDs through the bounded scheduler. Each worker
loads one definition from disk, checks it and destroys its recursive AST
iteratively. Results contain counts or errors, not retained bodies. Ordered
consumption reports the first body error in source order, independent of worker
count. Expression traversal and local-scope cleanup use explicit work stacks.

Initial coverage includes declaration kind/scope checking, function signatures,
primitive literals, rigid type variables, tuples, conditionals, blocks, local
bindings, monomorphic calls, return and direct list checking. Unknown expression
rules, polymorphic call solving, refinement proofs, synonym expansion,
mappings and scattered definitions fail explicitly. This is a partial checker:
it does not yet produce the complete typed AST required by the rewrite and Lean
backend passes. Successful checking of the supported subset still ends with
the CLI's explicit missing-backend error.

Validation uses `scripts/test-memory-limited.sh -p sail-ty -p sail-cli`: 162 tests
pass with each test binary limited to 128 MiB and swap disabled (33.1 MiB peak).
This includes actual positive/negative body checking with one and four workers,
source-order replacements and 10,000 nested `let` scopes checked and freed on
a 128 KiB thread stack. A positive fixture was also checked by OCaml Sail under
256 MiB without swap. These are not evidence of a complete ARM typecheck.

The memory bound is the resident declaration tables plus a bounded number of
loaded definitions, local scopes and worker temporaries. A single definition
must still fit RAM; there is no strict per-worker byte quota. Declaration tables
grow with exported signatures. Full ARM acceptance remains Sail to executable
Lean followed by a successful `lake build` under 16 GiB without swap (28 GiB
fallback).
