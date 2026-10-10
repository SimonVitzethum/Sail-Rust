# Lane 71 — payload-intern (RAM)
Branch: `lane/71-payload-intern` | Status: AKTIV

## Ziel
Lane-68-Abgrenzung schließen: TokenKind-/LitKind-/Kid-Payloads von `String` auf
Symbol/Interner umstellen (Snapshot-/Parser-Verträge per Difftest-Liste absichern).

## Aufgaben
- Umstellung + Vertrags-Doku (welche Snapshots betroffen), Tests als Code
  (NICHT ausführen, E3). `#![forbid(unsafe_code)]`.

## Done
- Payloads interniert (`TokenKind`/`LitKind`/`Kid`: `String` → `Symbol`;
  `lex(file, src, interner)`; `Symbol` +`Ord`; `TokenKind::Error` bleibt
  `String`), Verträge dokumentiert (`docs/payload-intern-contract.md`
  mit Difftest-Liste fürs Rig). Tests als Code, E3: kein Build/Test
  ausgeführt.
- Bewusst draußen: `NExpKind::Constant`/`Set.members`, `sail-nexp`
  `Id`/`App`-Namen, `VectorSubrange`/`Value`/`Config`/`Fixity.level`/
  `Pragma`/`Extern`, `sail-grammar`-Toks (siehe Vertrag §5).
