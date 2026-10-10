# Lane 125 — ty-sound (Bug-Hunt-Fixes)
Branch: `lane/125-ty-sound` | Status: AKTIV

## Ziel
HOCH-Befunde Nr. 5–9 + 13–15 (sail-ty): infer-Obligationen verwerfen,
val-def ohne Body-Check, freshen-Kollisionen (Gensym!), opaque_kenv-Maskierung,
KNat-Richtung, NTyp::Var-Substitution, Fuel-Verbrauch, DUMMY-Spans/FileId(0),
scattered-end-Namespaces, CachingSolver-Recency.

## Aufgaben
- Fixe minimal nach OCaml-Referenz (type_env.ml/initial_check.ml). Tests als Code
  (NICHT ausführen, E3). `#![forbid(unsafe_code)]`.

## Done
- Alle Punkte behoben oder begründet abgewiesen (Rig-Beweis offen).
