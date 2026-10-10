# Lane 38 — cli-frontend
Branch: `lane/38-cli` | Status: QUEUED

## Ziel
`sail`-CLI-Kompatibilität für den Lean-Pfad.

## Aufgaben
- Flags: `--lean`, `--lean-output-dir`, `--lean-force-output`, `-o`, `--memo-z3`-Äquivalent (bounded), `-dprofile`-Timings, `-v`.
- Exit-Codes + `--help`-Text wie OCaml.

## Done
- `arm-v9.4-a/Makefile gen_lean`-Aufruf läuft mit Rust-Binary (Drop-in).
