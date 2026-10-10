# Lane 124 — solver-sound (Bug-Hunt-Fixes)
Branch: `lane/124-solver-sound` | Status: AKTIV

## Ziel
HOCH-Befunde Bug-Hunt-Frontend Nr. 1–4 + 10–12 (sail-solver/sail-nexp):
Id/Var-Verwechslung (z3_backend), Symbol-pro-Interner (globale Keys!), Cache-Keys
namentlich statt kid.0, i128-Overflow (Zarith-Verlust), simplify-soundness
(0*x/a-a/mixed-sorts/Exp-Pos nur solver-gestützt oder gar nicht).

## Aufgaben
- Fixe minimal, Semantik aus OCaml-Referenz (Fundliste mit Stellen liegt vor —
  Haupt-Session gibt sie auf Anfrage; sonst sail-ty/solver.rs, z3_backend.rs,
  query.rs, simplify.rs, pred.rs, lean.rs lesen). Tests als Code (NICHT ausführen, E3).
- `#![forbid(unsafe_code)]`.

## Done
- Alle Punkte behoben oder begründet abgewiesen (Rig-Beweis offen).
