# sail-par-pipe

Memory-bounded parallel Parse → Check → Emit. All entry points use the same
`sail-shard` worker pool. Stage functions are supplied by the caller. The crate
does not implement missing compiler stages or whole-program dependency checks.

## Bounded streaming

`run_pipeline_streaming` accepts a lazy shard iterator and an ordered output
consumer. It runs the complete Parse → Check → Emit chain in one shared pool,
so `chain_workers()` is the total worker budget. Completed results count toward
the same credit window as queued inputs and running shards: at most
`max_inflight + chain_workers()` items are outstanding, even when an early shard
is slow. The consumer should write each result to disk and release its body.
Sink errors and worker panics cancel ingress and join workers without deadlock.

`run_pipeline` and `run_shard_parallel` remain collecting APIs. Their returned
vectors retain every output; bounded queues alone do not make these APIs
out-of-core. Cross-shard dependencies must be resolved before scheduling work
that requires them. Neither this orchestrator nor the streaming API implements
missing compiler stages.
