# Lane 02 — parser
Branch: `lane/02-parser` | Worktree: `sail-rs/worktrees/lane-02` (nach Merge löschen)

## Ziel
Sail2-Quelltext → `sail-ast`, ohne Gesamtdatei im RAM zu halten.

## Aufgaben
- `crates/sail-parse`: `lalrpop`-Grammatik aus OCaml-Menhir portieren; pro Datei streamend parsen.
- Pragmas (`file_start/file_end`, `include_start/end`) als Shard-Marker erhalten — wichtig für Lane 11.
- Fehler mit Span + Datei, für `sail-arm/arm-v9.4-a/src/*.sail` robust.

## Input/Output
- Input: `.sail`-Text. Output: `sail-asset`-AST + Dateiliste.

## Abgrenzung
- Keine Namensauflösung, kein Typecheck.

## Done
- Alle `sail/test/lean/*.sail` + `aarch64_small` parsen; `#![forbid(unsafe_code)]`.
