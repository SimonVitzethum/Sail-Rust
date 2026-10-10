# Lane 16 — lexer
Branch: `lane/16-lexer` | Status: QUEUED (nur 15 Worktrees aktiv, siehe `ACTIVE.md`)

## Ziel
Eigenständiger Sail2-Lexer (Abgrenzung vom Parser Lane 02).

## Aufgaben
- `crates/sail-lex`: Tokens, Kommentare `(* *)`, Bit-Literale (`0x`, `0b`), Strings, Positionen.
- Keine Grammatik, nur Tokenstrom + Fehlertoleranz.

## Done
- Lexer-Tests an `sail/test/lean/*.sail`; `forbid(unsafe_code)`.
