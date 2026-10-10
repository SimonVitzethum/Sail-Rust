# Lane 41 — multicore-lang-delta
Branch: `lane/41-lang-delta` | Status: QUEUED (Phase 2, erst nach Phase-1-Parität)

## Ziel
Sail-Sprachdelta für Multicore als Spec (kein Compiler-Umbau in dieser Lane).

## Aufgaben
- `SYNTAX.md`: `core_id`-Builtin, `shared`-Annotation (Memory/GIC/Timer, Rest per-Core),
  Atomics/Barrieren/IPI/WFE-SEV-Primitive, TLB-Shootdown-Hook, PSCI-Hooks (`reset`/`fetch`-Anbindung).
- Interleaving-Semantik dokumentieren; echte Parallelität nur im Emulator.
- E1 beachten (DECISIONS.md): Modell-Semantik bleibt Lean-Master.

## Done
- SYNTAX.md + 3 Beispiel-Schnipsel, Review durch Lane 29.
