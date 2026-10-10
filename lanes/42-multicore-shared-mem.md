# Lane 42 — multicore-shared-mem
Branch: `lane/42-shared-mem` | Status: QUEUED (Phase 2)

## Ziel
Shared-Memory/GIC/Counter-Trait (E1: Spiegel, kein eigenes Modell).

## Aufgaben
- `crates/sail-shared-mem`: pro-Core-State vs. shared State trennen, auf `sail-archsem` aufsetzen,
  little-endian, `OutOfMemoryRange`, Tag-Ablehnung wie SequentialState.
- GIC/Timer als erstklassige Geräte-Schnittstelle (Stub + Difftest-Vertrag).

## Done
- Difftests gegen Lean-Master-Semantik grün, E1-Konformität in Doku.
