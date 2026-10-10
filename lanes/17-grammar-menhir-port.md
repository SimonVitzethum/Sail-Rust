# Lane 17 — grammar-menhir-port
Branch: `lane/17-grammar` | Status: QUEUED

## Ziel
Menhir-Grammatik 1:1 nach `lalrpop` portieren.

## Aufgaben
- Grammatikdateien aus `sail/src/*.mly` sichten, Regeln für Exp/Pat/Typ/Def übernehmen.
- Konflikte dokumentieren, keine Semantic Actions mit Logik.

## Done
- `cargo build -p sail-parse` mit neuer Grammatik, Konflikte in `GRAMMAR.md`.
