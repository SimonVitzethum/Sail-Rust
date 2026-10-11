# ARM build objective

> Aktueller Fortsetzungsstand: [WORK_STATE.md](WORK_STATE.md). Seit dem letzten
> gepushten Vektorstand sind Mutable-/Loop- und Block-Scope-Fixes im Worktree.
> Der neueste 128-MiB-Fokustest scheitert durch OOM; er ist nicht grün.
> Die nachfolgenden ARM-Messungen stammen noch vom Stand vor diesen Änderungen.

The acceptance target is a complete ARM Sail build with the Rust compiler under
`systemd MemoryMax=16G MemorySwapMax=0`. A maximum of `28G` is an accepted fallback.
The confirmed reference is ARMv9.4-A, targeting Lean with a successful
`lake build` of the generated model. A parse census, stub output or synthetic stress test
is not a complete build. Lean acceptance must include compiling generated code
and contain no `sorry` proof holes.

## Verified foundations (2026-10-11)

- Tuple assignment now binds the RHS once before any target is written, matching
  `sail/src/lib/rewrites.ml::rewrite_tuple_assignments`. Self-assignments preserve
  register writes and effects. An executable regression checks calls and swaps.
- Undefined values use OCaml-style type-directed helpers, retaining symbolic bit
  widths and case-sensitive type names. Unsupported compound types stay explicit.
- The streaming scheduler limits queued, running and reordered outputs together.
  Its ordered consumer can spill results immediately. Inputs are pulled lazily,
  only after acquiring a credit. Errors and worker panics cancel ingress.
- The warm layer supports uncached concurrent writes and buffered reads, checks
  cached-read budgets before allocation, and prevents racing writers overwriting
  immutable blobs. The sequential driver no longer caches all completed bodies.
- Ordinary CLI invocations now expand includes and conditional macros in source
  order, spool raw definitions to disk, and convert them through the real grammar
  with bounded workers (default at most 16; `SAIL_RS_THREADS` overrides it).
  Fixity snapshots preserve compilation order. Completed ASTs with their local
  interners are stored as immutable blobs; resolution retains exported names and
  linkage summaries. The CLI rejects incomplete checking/emission with exit 1
  and never publishes a comments-only Lean project as a successful compilation.
- Attributes on the same line no longer swallow their following definition.
  Builtin argument sorts, refined `int`/`bool` names, tuple function arguments,
  numeric/bool aliases and operator folding have OCaml reference regressions.
- Binder kinds now follow their uses and constraints instead of defaulting to
  `Type`. Conflicting kinds fail explicitly; existential shadowing is local.
  Constructor signatures propagate through includes in source order. Workers
  share immutable copy-on-write signature maps, and already converted type
  declarations are reused rather than parsed twice. Earlier workers cannot see
  later declarations, and failed or outcome-local declarations do not escape.
- Parenthesized bitfield ranges and concatenations match the OCaml productions,
  including ARM's `(15..10 @ 9..0)`. Singleton type parentheses remain unique.
- Setter assignment becomes an application with the RHS appended once, matching
  OCaml's unit/tuple argument normalization and `setter` annotation. AST nodes
  store optional boxed annotations; expressions, patterns and top-level/nested
  definition wrappers retain structured attributes, docs and private visibility.
  Unannotated JSON remains compatible. Field/clause annotations and propagation
  through all later compiler passes still need completion.
- Expression lowering and expression-infix tree assembly now use explicit
  postorder work stacks. Converted children move once into their parent;
  folded temporary trees are freed iteratively. This removes the worker-stack
  overflow on ARM `ImpDefBool` without increasing thread stack sizes. The real
  22-source ARM frontend now reaches global name resolution; this is still not
  a typed compilation or Lean build. Other recursive type/pattern/serialization
  paths remain to be audited.
- Pattern-name collection uses an explicit work stack as well, with a regression
  covering 10,000 nested patterns on a 64 KiB thread stack. The grammar lexer
  distinguishes wildcard `_` from identifiers such as `_R`; repeated wildcard
  parameters no longer become duplicate value bindings.
- All 59 standard-library files are embedded, including nested concurrency and
  float libraries. A separately documented, unmodified Sail 0.17.1 `trace.sail`
  supplies the legacy ARM include; its license and source hash are retained.
- `scripts/test-memory-limited.sh` builds tests under 4 GiB and executes each
  binary under 128 MiB without swap. It includes a 512 MiB streaming workload and
  a 256 MiB rewrite/spill/read workload. These demonstrate memory mechanisms,
  not ARM compiler completeness. All test invocations use systemd limits.

## Reproducible acceptance gate

Run `scripts/arm-lean-gate.sh` (or `SAIL_ARM_MEMORY=28G` for the fallback).
It builds Rust under 4 GiB, obtains the 22 canonical ARM Lean inputs from the
model Makefile, and runs compilation, `lake build`, then
`scripts/CheckLeanNoSorry.lean` under the selected limit with swap disabled.
The final check imports every compiled project module and inspects elaborated
types and values, including opaque theorems and transitive imports, for
`sorryAx`. It catches holes from macros/tactics even if warnings are disabled;
comments and string literals do not cause false positives. Each invocation gets
a fresh output directory, so old artifacts cannot satisfy a failed run. Logs
are retained there.

`scripts/test-lean-no-sorry.sh` verifies the gate on Lean 4.29.0: a complete
theorem passes, while an opaque proof hole and a module importing that hole fail
after their own Lean compilation succeeds. All these runs use a 2 GiB/no-swap
systemd limit (1.5 GiB maximum). This validates the gate, not ARM emission.

## Remaining acceptance work

Latest validation (2026-10-11): the complete workspace has 1,163 passing tests
under `MemoryMax=4G MemorySwapMax=0` (401.6 MiB cgroup peak). The earlier focused
AST/parser/resolver/CLI gate has 226 passing tests with each binary limited to
128 MiB without swap (110.9 MiB maximum, including six 10,000-level trees on
a 128 KiB thread stack). Its separately limited build peaks at 533.8 MiB.

The canonical 22-input ARM frontend now converts, spools, reloads and resolves
31,267 definitions from 45 files using 16 workers on normal thread stacks.
Three legacy val redeclarations (`max_int`, `pow2`, `sub_bits_int`) produce
warnings; the later binding wins and existing extern metadata persists,
matching the OCaml reference and an executable differing-signature test.

Recursive AST JSON cannot reload ARM `ImpDefBool` under serde_json's nesting
limit. The new versioned flat format reconstructs and destroys all recursive
AST families iteratively, including annotation data and internal values.
Worker results contain only content IDs and sizes. See
[flat AST storage](flat-ast-storage.md) for its representation and limits.

The former `/tmp` spool was on tmpfs and therefore retained its data in RAM
with swap disabled. Spooling now uses the disk cache directory or
`SAIL_RS_SPOOL_DIR`; Linux RAM filesystems are rejected explicitly. The real
ARM frontend reads/writes 1,039,913,215 flat AST bytes under **512 MiB** with no
swap and reaches the explicit missing compiler stages (46.62 s, 512 MiB cgroup
peak). This demonstrates eviction of disk-backed data larger than the memory
limit. The regular **16 GiB** gate reaches the same point (44.66 s, 1.3 GiB
cgroup peak). The flat format currently uses roughly twice the space of the old
recursive JSON, and the ordered reader remains sequential. These are failed
compilations at the missing typechecker/backend, not full ARM build results.

The CLI now also runs a real, partial canonical typechecker. Declaration
histories preserve source-position lookup; bounded workers load bodies from
disk and report errors deterministically. Supported rules include ordinary
polymorphic calls, validated annotations, tuples, locals, conditions, enum
constants, union constructor signatures and literal refinements. Unproved
constructor conditions and unsupported rules fail explicitly. The dedicated
typechecker/CLI gate has **170 passing tests**, each binary under 128 MiB
without swap (38.9 MiB peak), including 10,000 nested calls/scopes/numeric terms
on small thread stacks. See [canonical typechecking](canonical-typechecking.md).

The large supported-subset gate checks **4,096 definitions and 1,052,672
expressions** with **16 workers under 128 MiB without swap**. It processes
567,919,927 AST bytes on disk (about 542 MiB); checking takes 1.36 s and the full
run 10.42 s. It deliberately requires the missing-backend error after checking.
It demonstrates actual disk-backed body checking, not complete ARM compilation.

The latest full ARM invocation under 16 GiB converts all 31,267 definitions and
passes canonical declaration checking, including the constrained memory-request
types. Constructor obligations now use simultaneous parameter substitution,
rigid scope identities, scheme/existential/source-position global assumptions,
and actual Z3 proofs through an iterative SMT-LIB bridge. Persistent subprocesses
share the compiler cgroup; a bounded memo cache and exclusive solver pool reuse
the existing solver architecture. Proof timeouts/errors fail closed.
Dependent calls now infer directly occurring numeric/Boolean parameters, retain
compound result types and prove quantifier constraints. Immutable bindings and
opaque scalar results receive distinct rigid singleton witnesses. Canonical
views avoid copying numeric trees; cached call instances and iterative
substitution cleanup handle 10,000 composed dependent calls on a 128 KiB stack.
Inline return annotations now synthesize complete signatures from argument
patterns, including argument-only binder-kind uses. Source-ordered overload
tables preserve OCaml candidate order and check all arguments, return context
and dependent obligations. Nested expansion and deep header cleanup use explicit
work stacks. The run passes `flow.sail`, inline signatures and operator lookup,
and proves the nonnegative-shift obligations in `_shl_int_general` using
separate true/false branch facts. Guards and nested conditions preserve borrowed
substitutions; facts cannot escape their immutable scope. Vector checking now
passes `slice_mask`, including `[bitone] : bits(1)`. The next failure is
`to_bytes_le` in `vector.sail`, requiring mutable declarations, loops and indexed
assignments. Explicit implicit argument views and general type-alias expansion
also pass; borrowed substitutions compose across interners and release mixed
chains iteratively. The parser now lowers `bitone`/`bitzero` to one-bit binary
literals, matching OCaml, and rejects them as numeric type arguments.
10,000-level alias and vector regressions pass on a 128 KiB stack. The run takes
50.69 s and peaks at 1.4 GiB without swap. The current focused parser/solver/
typechecker/CLI gate passes 322 tests under 128 MiB per binary without swap
(87.8 MiB peak); the separate canonical gate passes 35 tests. OCaml accepts the
15-definition positive vector fixture under 256 MiB without swap (67.2 MiB peak).
The earlier focused
solver/typechecker/CLI gate has 214 passing tests under 128 MiB per binary
(75.8 MiB peak), including 30 canonical tests, negative flow/overload cases and
10,000-level conditional scopes and inline tuple patterns. OCaml accepts the
positive flow fixture under 256 MiB without swap (130.9 MiB peak).
The focused solver/typechecker/CLI suite passes 203 tests with each binary limited
to 128 MiB, including 21 canonical tests and a stalled-child deadline/reaping
regression. The updated sixteen-worker out-of-core subset run passes at 128 MiB
without swap (1.38 s checking, 11.14 s overall). The CLI profiler's SMT counters
still need wiring to the canonical prover; their zeros are not proof-call counts.
No full ARM body check or Lake build has succeeded.

Full kind/type elaboration, all expression/mapping conversions, complete
annotation handling, typed rewrites and executable Lean body emission remain incomplete.
The E2E path also reports missing stages. Whole-program checking and generated
Lean must be verified on the full model. The bounded frontend holds a window of
definitions, not a strict byte quota: one large definition must still fit RAM,
and exported-name summaries grow with the program. Synthetic or partial
frontend peaks must not be reported as ARM build peaks. No successful full ARM
build, nor successful `lake build` of Rust-generated ARM, currently exists.
