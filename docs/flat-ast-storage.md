# Flat definition storage

The compiler's out-of-core unit is one definition and its local interner.
Recursive JSON of the canonical AST is unsuitable for this storage: ARM's
`ImpDefBool` exceeds serde_json's default nesting limit, and simply disabling
that limit would reintroduce recursive Rust deserialization on worker stacks.

`sail_ast::storage::DefinitionSnapshot` temporarily moves all recursive children
into a flat vector. Each record retains the original node metadata and payload;
child slots contain fixed, empty placeholders and refer to other records by
storage indices. These indices are independent of AST NodeIds and symbols.
Types, patterns, expressions, outcomes, numeric constraints, internal values
and structured annotation data use the same traversal. Exhaustive matches
make new AST variants require an explicit traversal decision.

Records are written in reverse index order, so every child precedes its parent
on disk. A reader parses bounded-depth JSON records, verifies a single typed,
acyclic tree and reconstructs it iteratively. No thread-stack enlargement or
unbounded serde recursion is needed. The representation is internal and
versioned separately from human-readable AST JSON snapshots.

The frontend workers spool flat definitions and return only content IDs and
sizes. The ordered consumer reads each definition from the warm layer before
resolution, then releases it with the same iterative traversal. Completed AST
bodies therefore do not stay in the scheduler's ordered result queue.

The spool defaults to `$XDG_CACHE_HOME/sail-rs/spool`, or
`$HOME/.cache/sail-rs/spool` if XDG_CACHE_HOME is unset. Without either variable
it uses `target/sail-rs-spool` in the working directory. `SAIL_RS_SPOOL_DIR`
overrides the parent directory; each invocation creates and removes its own
isolated subdirectory. On Linux the compiler checks `/proc/self/mountinfo`
and rejects RAM filesystems. In particular, `/tmp` on tmpfs cannot serve as
out-of-core storage with swap disabled.

The current memory bound is still a definition window rather than a byte quota.
One definition, its flat records and its encoded buffer must fit in a worker's
memory; symbol summaries remain resident. This design does not establish the
full ARM compilation memory bound by itself.
