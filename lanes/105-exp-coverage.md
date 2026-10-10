# Lane 105 — exp-coverage (22/30 Exp-Arten)
Branch: `lane/105-exp-coverage` | Status: AKTIV

## Ziel
Reviewer-5-HIGH: MiniExp um fehlende Exp-Arten erweitern (Match/For/Loop/Vector/
Struct/Assign/Assert/Sizeof u.a., OCaml Ast.ml:242-277 als Liste) — Typen + Konverter,
kein Checker-Umbau.

## Aufgaben
- Varianten + conv-Arme, Tests als Code (NICHT ausführen, E3). `#![forbid(unsafe_code)]`.

## Done
- Alle Exp-Arten darstellbar.
