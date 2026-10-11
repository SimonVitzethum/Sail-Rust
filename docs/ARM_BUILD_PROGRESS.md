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
- `scripts/test-memory-limited.sh` builds tests under 4 GiB and executes each
  binary under 128 MiB without swap. It includes a 512 MiB streaming workload and
  a 256 MiB rewrite/spill/read workload. These demonstrate memory mechanisms,
  not ARM compiler completeness. All test invocations use systemd limits.

## Remaining acceptance work

The CLI's ordinary Lean path still emits a marked frontend stub. The E2E path
reports missing stages. Preprocessing/includes, full typed AST integration,
whole-program checks and executable Lean emission must be wired up and verified
on the full reference model. A successful exit from the stub path must never be
recorded as successful ARM compilation. Synthetic memory peaks must not be
reported as ARM build peaks. No measured full ARM build currently exists.
