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
