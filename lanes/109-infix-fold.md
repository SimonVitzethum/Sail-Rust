# Lane 109 — infix-fold (Präzedenz/Elaboration)
Branch: `lane/109-infix-fold` | Status: AKTIV

## Ziel
Reviewer-5-MEDIUM-HIGH: Infix-Folding/Präzedenz (infix_parser.mly, 468 Zeilen) —
InfixChain elaborieren statt roh speichern (ARM nutzt viele Infix-Ops).

## Aufgaben
- Elaboration (Präzedenz-Tabelle aus OCaml übernehmen), Tests als Code
  (NICHT ausführen, E3). `#![forbid(unsafe_code)]`.

## Done
- Infix-Bäume korrekt (Rig-/Korpus-Beweis offen).
