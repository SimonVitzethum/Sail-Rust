# Lane 84 — parpipe-impl (Multithreading Ende-zu-Ende)
Branch: `lane/84-parpipe-impl` | Status: AKTIV — Ziel: ARM multithreaded.

## Ziel
Lane-69-Skizze zur Implementierung: 3-Pool-Pipeline (Parse→Check→Emit) + Merge,
Backpressure-Default 8, Worker=Kerne, deterministische Ordnung, Fehlerisolation.

## Aufgaben
- Implementieren auf Lane-11/43-Schnittstellen (lesen), Tests als Code
  (NICHT ausführen, E3). Nur std+rayon/crossbeam. `#![forbid(unsafe_code)]`.

## Done
- Pipeline implementiert, Ordnung/Isolation per Test-Code bewiesen (Rig folgt).
