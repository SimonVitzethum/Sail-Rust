# Lane 98 — grammar-family (nullable-Listen flächendeckend)
Branch: `lane/98-grammar-family` | Status: AKTIV

## Ziel
Restliche ~25 LALRPOP-Konfliktstellen nach E4-Muster (strikt, rechts-rekursiv,
explizite Trailing-Arme) bereinigen: Enum/ExpList/CaseList/StructFields (nullable-Listen),
IndexRange/@, AtomicIndexRange/.., LoopExp/by/in, BlockItem/in, TypSchm/effect.
Lane-87-Fix (RDefBody) als Vorlage lesen.

## Aufgaben
- Fix als Code, Tests als Code (NICHT ausführen, E3 — Rig-Loop über Haupt-Session,
  bei Fehlschlag Follow-up). `#![forbid(unsafe_code)]`.

## Done
- process_root läuft durch (Rig-Beweis offen).
- Lane-98-Stand (E3, Tests als Code, kein Build): RDefBody (Lane-87-Form,
  dieser Branch war davor divergiert) + Enum/ExpList/CaseList/StructFields
  strikt (rechts-rekursiv, Trailing-`,`, `?`-Use-Sites wo der Port `{}`
  akzeptierte) + explizite Optionsschwanz-Arme (IndexRange/@,
  AtomicIndexRange/`..`, LoopExp 4× by/in, TypSchm ±effect) + bare
  `let`/`var` als `Block`-Arme mit `;` (BlockItem nur noch Exp).
  Sprache jeweils unverändert. Rig-Beweis + Restfamilie
  (TypList/NumList/PatList/… Listen, `(...)+`-Makros, else/dangling-Reste)
  offen → Follow-up.
