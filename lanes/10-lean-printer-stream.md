# Lane 10 — lean-printer-stream
Branch: `lane/10-lean-printer` | Worktree: `sail-rs/worktrees/lane-10` (nach Merge löschen)

## Ziel
Streaming-Lean-Printer (Ersatz für akkumulierendes `doc_defs_rec`/`pp_ast_lean`).

## Aufgaben
- `crates/sail-lean`: `doc_typ/doc_nexp/doc_pat/doc_exp/doc_typdef/doc_fundef` als Writer (`fmt::Write` direkt in Datei), nie `PPrint.document`-Gesamtbaum.
- Monaden-Abbildung: rein → Wert, stateful → `SailM ... do`, early-return → `SailME`/`ExceptM`, Loops → `while/for`-Idiom; `match_bv` vs `match`; nicht-Übersetzbares als harter Fehler mit Span (kein stilles `sorry`).
- Zeilenbreite/Optionen wie `-lean-line-width`, `noncomputable/partial`-Flags.

## Done
- Byte-identischer Output an mind. 5 `test/lean/expected.lean`-Fällen.
