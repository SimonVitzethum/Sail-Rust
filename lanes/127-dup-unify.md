# Lane 127 — dup-unify (Bug-Hunt-Fixes)
Branch: `lane/127-dup-unify` | Status: AKTIV

## Ziel
Integrations-Duplikate vereinen (Top zuerst): Solver-Trait ×2, Fingerprint ×2,
NExp ×3 (nicht alle auf einmal — Owner-Tabelle + kleinste zuerst: Fingerprint,
Op-Tabellen, IntKind/Facts, LoopKind, Shard, DiskCache-Namen, CachingSolver),
Span-Duplikat NICHT anfassen (Serde-Drift dokumentiert!), tote Deps raus,
Re-Exports nachziehen.

## Aufgaben
- Je Duplikat: eine Wahrheit + Dep-Verdrahtung + Migrationsnotiz. Tests als Code
  (NICHT ausführen, E3). `#![forbid(unsafe_code)]`. Fremde Logik nicht umbauen.

## Done
- Top-Duplikate vereint, Rest mit Owner-Tabelle.
