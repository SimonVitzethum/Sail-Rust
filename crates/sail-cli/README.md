# sail-cli (Lane 38)

OCaml-kompatibles CLI-Frontend (`sail-rs`-Binary) für den Lean-Pfad.

- Flags: `--lean`, `--lean-output-dir`, `--lean-force-output`, `-o`,
  `--lean-line-width`, `--lean-extern-type`, `-v`, `--help` im
  OCaml-Sail-Stil. Ein/zwei `-` sowie `-`/`_` sind äquivalent
  (`-memo_z3` == `--memo-z3`), `--flag=wert` funktioniert,
  `-oarmv9` wird wie in OCaml abgelehnt. Übrige Backend-Flags
  (C/Coq/Lem/SV/…) werden für Drop-in-Kompatibilität akzeptiert und ignoriert.
- `--memo-z3`-Äquivalent: strikt **bounded** (Default 10000 Einträge /
  64 MiB, FIFO-Eviction; `--memo-z3-max-entries`, `--memo-z3-max-bytes`,
  `--no-memo-z3`, `--memo-z3-path`). Disk-Persistenz folgt mit Lane 06.
- `-dprofile`: Timings-Log auf stderr (Lane-14-Stil),
  `[profile] phase="…" wall_s=… rss_peak_mb=… smt_calls=… smt_s=…`.
- Exit-Codes: `0` Erfolg (auch `--help`/`-v`/`--dry-run`), `1` Usage-/Sail-Fehler,
  `2` I/O-fatal (Eingabe fehlt, Ausgabeverzeichnis-Konflikt).

Drop-in (`arm-v9.4-a/Makefile`, Dry-Run, kein Full-Build):

```sh
cargo build -p sail-cli
make -C <sail-arm>/arm-v9.4-a gen_lean SAIL="$PWD/target/debug/sail-rs SAIL_RS_DRY_RUN=1" -n
# oder direkt:
SAIL_RS_DRY_RUN=1 ./target/debug/sail-rs -dprofile -verbose 1 -memo_z3 \
  -lean --lean-force-output -lean_output_dir lean -o armv9 …
```

`Cargo.lock` wird nie committet (siehe `.gitignore`).

## OoC-Verdrahtung (Lane 62, E4)

`src/ooc.rs` + `src/e2e.rs` (`stream_rewrite`, `run_with_ooc`): Rewrite-Pässe
laufen shardweise hinter einer Barriere (höchstens ein heißer Shard),
fertige Shards werden warm gespillt, Solver-Anfragen kalt memoisiert
(`OocDriver::memo_prove`), Cross-Shard-Wissen nur als Summaries (≤ 64 KiB).

Herkunft (gelesen, nicht gedoppelt — in diesem Branch-Stand nicht
vorhanden): `sail-ooc` (Lane 60, `crates/sail-ooc` im Haupt-Repo) mit
`PhaseBarrier`/`WarmLayer`/`MemoSpills`/`SummaryTable`/`ContentId`. Die Hooks
(`OocHooks`) nehmen nur opake Content-Hash-Strings; sobald `sail-ooc` landet,
bekommt es ein `impl OocHooks` — der Treiber ändert sich nicht. E3: Tests als
Code (`#[cfg(test)]`), kein Build/Test aus Lane-Agents.
