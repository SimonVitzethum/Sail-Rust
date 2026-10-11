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

Current coverage includes declaration kind/scope checking, function signatures,
validated annotations, rigid binders, tuples, conditionals, blocks, local bindings,
monomorphic calls and unconstrained Type-polymorphic calls, return and direct
list checking. Literals retain integer/Boolean singleton types and exact binary
or hexadecimal widths. Numeric/Boolean type arguments can be proved equal
syntactically using an explicit stack and interner-aware binder identities.
Enum patterns are constants, register shadowing is rejected, and union
constructors have generated canonical function signatures. Recursive record
and union types are rejected, matching OCaml's `forbid_recursive_types`.

Unknown expression rules, dependent call solving, general refinement proofs,
constrained constructor applications, synonym expansion,
mappings and scattered definitions fail explicitly. This is a partial checker:
it does not yet produce the complete typed AST required by the rewrite and Lean
backend passes. Successful checking of the supported subset still ends with
the CLI's explicit missing-backend error.

Validation uses `scripts/test-memory-limited.sh -p sail-ty -p sail-cli`: 170 tests
pass with each test binary limited to 128 MiB and swap disabled (38.9 MiB peak).
This includes actual positive/negative body checking with one and four workers,
source-order replacements, 10,000 nested `let` scopes, 10,000 nested polymorphic
calls and 10,000 nested numeric expressions on 128 KiB thread stacks. A larger
positive fixture covering polymorphism, annotations, enums, unions, bit widths
and rigid dependent identities was checked by OCaml Sail under 256 MiB without
swap (49.9 MiB peak). These are not evidence of a complete ARM typecheck.

`scripts/typecheck-ooc-gate.sh` generates a supported-subset workload and checks
all 4,096 definitions and 1,052,672 expressions with 16 workers under
**128 MiB without swap**. The flat AST store contains **567,919,927 bytes**
(about 542 MiB), over four times the cgroup limit. The run peaks at 128 MiB;
body/declaration checking takes 1.36 s and the full frontend/checking run 10.42 s.
The earlier eight-worker run also passes (1.98 s checking, 9.86 s total).
The script requires the expected counts, a workload larger than the memory
limit and the explicit missing-backend failure. It does not accept this partial
compilation as ARM or Lean success.

The current full ARM gate reaches an explicit missing proof during declaration
checking: `Mem_read_request` in
`concurrency_interface/read_write_v1.sail`. Its constructor conditions must be
instantiated and discharged from the calling scheme's assumptions. Kind/scope
checking alone cannot establish this, so the checker rejects it until the
canonical dependent solver bridge exists. The latest full-model invocation
still exits 1, peaks at 1.3 GiB under the 16 GiB limit and never reaches Lake.

The memory bound is the resident declaration tables plus a bounded number of
loaded definitions, local scopes and worker temporaries. A single definition
must still fit RAM; there is no strict per-worker byte quota. Declaration tables
grow with exported signatures; synthesis caching is local to one loaded body.
Retaining signatures and annotation types still uses canonical AST `Clone`;
deep type copying and other recursive compiler paths require a further audit.
Full ARM acceptance remains Sail to executable
Lean followed by a successful `lake build` under 16 GiB without swap (28 GiB
fallback).
