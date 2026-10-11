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

Constrained type constructors now retain their canonical quantifiers and prove
every instantiated condition. Simultaneous Int/Bool substitutions target the
caller's rigid scope; scheme, existential and preceding global constraints
provide assumptions. Existential shadowing creates fresh proof identities and
restores the previous assumptions on exit. Annotation interners retain the
function's binder identities. Unknown obligations are errors.

Dependent calls now infer directly occurring Int/Bool parameters, retain
compound result refinements and prove repeated-argument equalities and
quantifier constraints. Scalar substitutions are borrowed views into canonical
types with their own interners/scopes; sibling and recursive calls have separate
instantiations. Shared call instances and synthesis caches avoid repeatedly
expanding nested results. Releasing substitution chains is iterative. A test
checks 10,000 composed `int('n) -> int('n + 1)` calls on a 128 KiB stack.

Plain `int`/`bool` bindings and expression results open fresh rigid singleton
witnesses. Reusing one immutable binding retains its witness; unrelated values
and opaque function results have distinct witnesses. This permits reflexive
equality and calls such as `not_bool(eq_bool(x, y))` without inventing a constant
value. Conditional and clause-guard predicates now refine dependent proofs.
The true branch borrows the singleton Boolean constraint with its substitutions;
the false branch uses its negation without cloning the numeric/Boolean AST.
Facts follow immutable local scopes and do not escape to siblings, outer scopes
or other clauses. A condition is checked before its branch facts can be used.
Mutation and short-circuit effect elaboration still require further rules.

The solver bridge emits flat, validated SMT-LIB with an explicit work stack;
decimal constants retain arbitrary precision. It reuses the existing exclusive
solver pool and bounded 128-entry memo cache. Each active solver owns a persistent
Z3 subprocess, reset before every request. Parsing errors, timeouts, missing Z3
and broken processes cannot prove an obligation. A separate I/O thread allows
the driver to kill a stalled child, including during a blocked write. All child
processes inherit the compiler's systemd memory and swap limits. Z3 must be on
PATH. Powers, general numeric/constraint applications and type-valued equality
currently require further elaboration and fail explicitly at this bridge.

Inline annotations now synthesize source-position function signatures from
typed, literal and tuple argument patterns; their stored type is the return
type, as in OCaml. With a separate `val`, duplicate inline quantifiers and
incompatible returns are rejected. The parser infers binder kinds from argument
annotations as well as returns. Header cleanup and tuple-pattern inference use
explicit stacks, tested at 10,000 levels on a 128 KiB stack.

Explicit `implicit('n)` parameters are viewed as `int('n)` in function argument
positions, following OCaml's `implicit_to_int`. The view preserves the original
scalar identity without copying its AST. Return positions are not converted:
OCaml rejects `int('n)` as an `implicit('n)` result, and the regression does too.
Omitted implicit arguments still require materialization. All 31 canonical
tests pass under 128 MiB without swap (88.6 MiB peak); OCaml accepts the positive
implicit fixture under 256 MiB without swap (30.8 MiB peak).

Overload declarations retain source-position histories and append candidates in
OCaml order. Nested lists expand iteratively with cycle detection. Calls check
every argument, the expected return when available, and instantiated dependent
constraints before selecting a candidate. Unsupported candidate elaboration
remains an error. Context propagation through nested ambiguous calls is not yet
complete.

Unknown expression rules, arithmetic inference of nontrivial call parameters,
general refinement subsumption, synonym expansion,
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

The latest full ARM gate passes the declaration phase, including the constructor
conditions of `Mem_read_request` and `Mem_write_request`, inline signatures and
operator overload resolution. Branch proofs now pass both general shift
functions in `arith.sail`. The next failure is `slice_mask` in `vector.sail`,
where an `implicit('n)` argument must behave as an integer singleton.
The run takes 51.80 s, peaks at 1.3 GiB under the 16 GiB/no-swap limit
and still exits 1 before Lean/Lake.
The focused solver/typechecker/CLI suite has 203 passing tests under 128 MiB per
binary, including 21 canonical tests. It covers source-position global
assumptions, cross-interner annotations and recursive/sibling call separation.
The solver bridge also
handles 10,000 nested numeric nodes on a 128 KiB stack and rejects an SMT-injection
literal. These remain partial compiler results.

The updated sixteen-worker out-of-core subset gate still checks all 1,052,672
expressions under 128 MiB without swap (1.38 s checking, 11.14 s overall). The
canonical prover's SMT timing/call counters are not yet connected to the CLI
profiler; its printed zero counters do not imply that no Z3 requests occurred.
Use phase wall time and the systemd cgroup peak for these measurements.

The solver/typechecker/CLI gate now has 214 passing tests under 128 MiB per
binary without swap (75.8 MiB peak), including 30 canonical tests. Flow tests
cover nested numeric bounds, scalar equality, Boolean predicates, alias scopes,
clause guards, invalid conditions and rejection of leaking branch assumptions.
10,000 nested conditional fact scopes check and release on a 128 KiB stack.
The CLI checks a positive and negative dependent overload/branch fixture with
one and four disk-backed workers. OCaml Sail accepts the positive flow fixture
under 256 MiB without swap (130.9 MiB peak; an expected exhaustiveness warning
for its guarded single clause).

The memory bound is the resident declaration tables plus a bounded number of
loaded definitions, local scopes and worker temporaries. A single definition
must still fit RAM; there is no strict per-worker byte quota. Declaration tables
grow with exported signatures; synthesis caching is local to one loaded body.
Retaining signatures and annotation types still uses canonical AST `Clone`;
deep type copying and other recursive compiler paths require a further audit.
Full ARM acceptance remains Sail to executable
Lean followed by a successful `lake build` and an elaborated no-sorry check under
16 GiB without swap (28 GiB fallback).
