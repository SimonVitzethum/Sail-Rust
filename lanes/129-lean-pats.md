# Lane 129 — lean-pats (Bug-Hunt-Fixes)
Branch: `lane/129-lean-pats` | Status: AKTIV

## Ziel
Lean-Emission MITTEL/NIEDRIG 7–13 + 15–23: fixup_match_id, P_as-unwrap,
lits_only-strikt, P_typ-Drop→Ascription, print_lit-#w, Num-Klammern,
sanitize/kid-Präfix, ctor-renames/Shadowing, Not→Hard-Error, ArchFieldsV1-voll,
untranslatable-Semantik (2×), Bool-Args-Drop, String-Escapes, mapping_last,
vector-Arg-Reihenfolge.

## Aufgaben
- Gegen OCaml-Referenz angleichen (Fundliste mit Zeilen). Tests als Code
  (NICHT ausführen, E3). `#![forbid(unsafe_code)]`.

## Done
- Alle Punkte behoben oder begründet abgewiesen (Rig-Beweis offen).
