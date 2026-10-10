# Lane 69 — par-pipe (Multithreading durchgehend)
Branch: `lane/69-par-pipe` | Status: AKTIV

## Ziel
Shard-parallele Pipeline Parse→Check→Emit mit bounded Queues (auf Lane 11/43/62 aufsetzend,
lesen nicht doppeln): ein Shard pro Worker, Backpressure, deterministische Merge-Reihenfolge.

## Aufgaben
- Orchestrierungs-Code (noch kein Treiber-Umbau fremder Crates), Tests als Code
  (NICHT ausführen, E3). `#![forbid(unsafe_code)]`, nur std+rayon/crossbeam.

## Done
- Pipeline-Skizze + Doku, Verifikation via Test-Rig (später).
