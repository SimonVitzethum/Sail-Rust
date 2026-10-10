# Lane 65 — serde-env (Lane-61-Follow-up)
Branch: `lane/65-serde-env` | Status: AKTIV

## Ziel
JSON-Roundtrip für Env-Typen (Lane-61-Follow-up): serde-Derives auf Env
(KindEnv/TypeEnv) + Roundtrip-Tests als Code (NICHT ausführen, E3).

## Aufgaben
- Minimalinvasiv, `#![forbid(unsafe_code)]`, MSRV 1.75 beachten (serde-Version pinnen).

## Done
- Roundtrip-Test vorhanden, NEEDED-Eintrag geschlossen.
