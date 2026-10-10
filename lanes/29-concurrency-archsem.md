# Lane 29 — concurrency-archsem
Branch: `lane/29-archsem` | Status: QUEUED

## Ziel
Concurrency-Interface V1/V2 (`ArchSem`, `Arch`-Instanz, `mem_acc`-Familie).

## Aufgaben
- `doc_instantiations_v1/v2`-Port, `interface.sail`/`mem.sail`-Anbindung.
- Kein Multicore-Emulator — nur Instanz-Gen (Multicore bleibt Isla/RMEM-Sache).

## Done
- `tiny-arm` Arch-Instanz baut.

## Single-Master-Regel (bindend, siehe `../DECISIONS.md` E1)
- Lean ist Master (mit Beweisen), Rust ist Spiegel — keine eigene Speichermodell-Semantik in dieser Lane.
- Jede Abweichung per Differentialtest belegen; Drift blockiert Merge.
- Gilt auch gegenüber Gabbros arm/Arm/Mem-Track: kein zweites Modell.
