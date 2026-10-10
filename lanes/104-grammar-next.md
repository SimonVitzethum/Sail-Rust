# Lane 104 — grammar-verify-loop (Rig-Evidenz)
Branch: `lane/104-grammar-next` | Status: AKTIV

## Ziel
Nächste Rig-Konfliktstelle nach Lane 103 (neuestes limited-run-*.log lesen!):
exakte State-Meldung + Menhir-Regel danebenlegen, E4-Fix, Tests als Code.

## Aufgaben
- NUR die belegte Stelle (+ offensichtliche Geschwister). KEINE Builds (E3, Rig-Loop).
  `#![forbid(unsafe_code)]`.

## Done
- Rig `limited-run-20261010-193612` (`sail.lalrpop:316:5-316:62`):
  `C : {}` zweibaumig (`Id(C,NSet([]))` vs `AnonRecord(C,[])`;
  Geschwister: `TypeUnions`-Tail, `union`/`newtype` — eine Regel).
  Menhir `parser.mly:1136` verlangt nicht-leere `struct_fields`
  (KOpt startet bei `Kid`/`(`, keine weitere `{...}`-Ueberlappung):
  Anon-Arm strikt (`StructFields` ohne `?`), `C:{}` nur noch
  `Id(C,NSet([]))`. Sprache unveraendert (leeres `NSet`,
  `struct={}`, `union={}` weiter ok); Tests als Code (E3, kein Build).
  Rig-Beweis offen (Haupt-Session).
