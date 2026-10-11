# ARM build objective

The acceptance target is a complete ARM Sail build with the Rust compiler under
`systemd MemoryMax=16G MemorySwapMax=0`. A maximum of `28G` is an accepted fallback.
The confirmed reference is ARMv9.4-A, targeting Lean with a successful
`lake build` of the generated model. A parse census, stub output or synthetic stress test
is not a complete build. Lean acceptance must include compiling generated code.

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
model Makefile, and runs compilation and then `lake build` under the selected
limit with swap disabled. Each invocation gets a fresh output directory, so
old artifacts cannot satisfy a failed run. Logs are retained there.

## Remaining acceptance work

Latest validation (2026-10-11): the complete workspace has 1,128 passing tests
under `MemoryMax=4G MemorySwapMax=0` (716.6 MiB cgroup peak). The additional
memory gate executes 368 frontend/OOC/scheduler/rewrite tests under 128 MiB
per binary, including the large ARM definition, 10,000-pattern traversal and
512/256 MiB synthetic workloads. All pass; test binaries peak at 13.9 MiB
(the separately limited build step peaks at 506.1 MiB).

The latest canonical 22-input ARM gate completes grammar/core-AST conversion
without enlarging worker stacks and reaches name resolution at 680.4 MiB
cgroup peak under the 16 GiB limit. It fails with three resolver diagnostics,
starting with duplicate `max_int` val specs in the library and ARM prelude.
The OCaml reference `type_env.ml::add_val_spec` permits legacy duplicate
function declarations with a warning and updates their binding; the Rust
resolver currently rejects them. This is the next semantic mismatch to fix.
These figures measure a failed frontend run, not a complete ARM compilation.

Full kind/type elaboration, all expression/mapping conversions, complete
annotation handling, typed rewrites and executable Lean body emission remain incomplete.
The E2E path also reports missing stages. Whole-program checking and generated
Lean must be verified on the full model. The bounded frontend holds a window of
definitions, not a strict byte quota: one large definition must still fit RAM,
and exported-name summaries grow with the program. Synthetic or partial
frontend peaks must not be reported as ARM build peaks. No successful full ARM
build, nor successful `lake build` of Rust-generated ARM, currently exists.
