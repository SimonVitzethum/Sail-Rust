# sail-shard (Lane 11)

Datei-Sharding + Inkrementalitaet fuer den reinen Rust-Sail-Compiler.
`#![forbid(unsafe_code)]`, low-RAM (bounded Queue), maximal multithreaded.

## Shards

- Grenzen aus `file_start "name"` / `file_end`-Pragmas (`split_by_file_markers`).
  Namenloser Text: fuehrend `preamble`, danach `postamble[#n]` (eindeutige IDs).
- Extra-Splits pro Instr-Datei (`extra_split`, `max_lines` pro Chunk) fuer
  `instrs32`, `instrs64`, `sve`, `sme`, `v8_base`, `sysregs_autogen`.
- Kanonische ARMv9-Reihenfolge: `armv9_shard_plan` / `ARMV9_SHARD_ORDER`.

## Fingerprint

`fingerprint(content, pass_version)` = blake3 ueber
`len(version) || version || len(content) || content`.
Stabil; jede Aenderung an Inhalt oder Pass-Version aendert den Hash.
`fingerprint_all` nutzt rayon.

## Scheduler

- `Scheduler::run` / `run_ordered`: crossbeam-Arbeits-Channel mit Kapazitaet
  `max_inflight`, `num_workers` Threads.
  Produzent blockiert bei voller Queue -> wartende Eingaben O(max_inflight).
  Ergebniskanal unbegrenzt (Deadlock-Freiheit; Ergebnisse muessen ohnehin alle
  gehalten werden).
- Default: `max_inflight = 8`, Worker = Kerne.
- `Scheduler::run_rayon`: datenparalleles Map im globalen rayon-Pool.

## Inkrementalitaet

`IncrementalCache`: `plan()` -> pro Shard `Skipped`/`Dirty`;
`mark_built()` uebernimmt nur erfolgreiche Builds.
Zweitlauf ohne Aenderung = alles `Skipped`.
`serialize()` / `deserialize()` persistieren den Cache als Text
(`sail-shard-cache-v1`, eine Zeile pro Shard) fuer prozessuebergreifende Laeufe.
