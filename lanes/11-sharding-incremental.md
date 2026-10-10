# Lane 11 — sharding-incremental
Branch: `lane/11-sharding` | Worktree: `sail-rs/worktrees/lane-11` (nach Merge löschen)

## Ziel
Datei-Sharding + Inkrementalität (nur geänderte Shards neu).

## Aufgaben
- Shard-Grenzen aus `file_start/file_end` + Extra-Splits pro Instr-Datei (`instrs32/64/SVE/SME`, `v8_base`, `sysregs_autogen`).
- Fingerprints pro Shard (Hash über Input+Pass-Version), Skip bei Unverändert, bounded Queue (z.B. 8 Shards inflight).
- Parallel-Scheduler (`rayon`/`crossbeam`) über Shards.

## Done
- Zweitlauf ohne Änderung = alles skipped; ARMv9-Shardliste dokumentiert.
