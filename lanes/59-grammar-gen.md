# Lane 59 — grammar-generating (Follow-up zu Lane 17)
Branch: `lane/59-grammar-gen` | Status: QUEUED (nach Welle 3 starten)

## Stand
Lane 17 lieferte Skelett (Lexer + `sail.lalrpop` als Doku, gemergt). Direkter Menhir-Port
scheitert an 4 LALR-Konflikten (Menhir-Shift-Präferenz vs. LALRPOP): dangling-else,
dangling-in (2×, selbstverschuldet durch optionales `in`), nullable Block + `with`.
Validierte Fix-Designs liegen vor (Scratch `/tmp/opencode/laltest`, LALRPOP 0.22, exit 0):
E3c (if-Faktorisierung ohne `E ::= R`), E4 (`in`-Pflicht in ExpLet/ExpVar, Block strikt).

## Aufgaben
- (a) E3c+E4 in `sail.lalrpop` übernehmen, (b) echte Verdrahtung (`lalrpop` Build-Dep +
  `lalrpop_mod!` + `parse_*`), (c) Korpus-Test über Sail-Libs (D1/D2-Abweichungen bewerten),
  (d) GRAMMAR.md auf „generierend" drehen. E3 beachten (Tests nur via Test-Rig).

## Done
- `cargo test` (Rig) grün inkl. Korpus-Test, keine LALR-Konflikte.
