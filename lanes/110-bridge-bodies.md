# Lane 110 — bridge-bodies (DEF-Rümpfe)
Branch: `lane/110-bridge-bodies` | Status: AKTIV

## Ziel
Bridge-`deferred`-Reste senken: Type/Let/Constraint/Register/Measure/Outcome/
Instantiation-Rümpfe auf sail-ast-DefKind heben (statt Rohtext-Sidecar).

## Aufgaben
- Konverter je Def-Art, Tests als Code (NICHT ausführen, E3). `#![forbid(unsafe_code)]`.

## Done
- deferred nur noch für wirklich Unbekanntes.
