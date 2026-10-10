# Lane 88 — corpus-const (E0015×26)
Branch: `lane/88-corpus-const` | Status: AKTIV

## Ziel
26× E0015 in sail-arm-corpus: Closure `eq(...)` in const-Kontext → const fn oder
Match-Struktur (Rig-Log: lib.rs:108+). Mechanischer Fix, Semantik identisch.

## Aufgaben
- Fix + Tests als Code (NICHT ausführen, E3). `#![forbid(unsafe_code)]`.

## Done
- Kein E0015 mehr (Rig-Beweis offen).
