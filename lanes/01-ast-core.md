# Lane 01 — ast-core
Branch: `lane/01-ast-core` | Worktree: `sail-rs/worktrees/lane-01` (nach Merge löschen)

## Ziel
Reine, `forbid(unsafe_code)`-taugliche AST-Kerntypen für Sail2. Keine Logik, nur Daten.

## Aufgaben
- `crates/sail-ast`: alle Sail2-Knoten (Typ, NExp, Constraint, Pat, Exp, Def) als Enums/Structs mit generationalen IDs (`SlotMap`-Stil, nur Indizes, keine Pointer).
- String-Interning (`lasso` o.ä.), `Span`-Typ, Serde für Snapshots.
- Kein OCaml-Copy-Paste der Logik, nur Struktur aus `sail/src/libsail/ast.ml` übernehmen.

## Schnittstellen (Output für andere Lanes)
- `NodeId`, `Span`, `Interner`, `Ast` + JSON-Roundtrip.

## Abgrenzung
- Kein Parser, kein Typecheck, kein Druck.

## Done
- `cargo test -p sail-ast` grün, `#![forbid(unsafe_code)]`, Clone+Send+Sync wo nötig.
