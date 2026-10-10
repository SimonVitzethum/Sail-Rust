# Bindende Entscheidungen (sail-rs)

## E1 — Single Master fürs Speichermodell (Lean = Master, Rust = Spiegel)

- **Master:** Lean — `lean-sail` (`Sail/ArchSem*.lean`, `Sail/ConcurrencyInterfaceV1.lean`), `ArmExtras.lean`,
  generiertes ARM-Modell — **mit Beweisen**. Nur hier darf sich die Speichermodell-Semantik ändern.
- **Spiegel:** Rust — Lane 29 (`concurrency-archsem`) und zugehörige Crates. Keine eigenständige Semantik,
  kein zweites Modell, das driften kann.
- **Sync-Mechanismus:** Differentialtests (Lane-13-Harness + Lane-29-Mem/ArchSem-Vergleiche).
  Drift = roter Test = Merge-Blocker.
- **Gilt auch für Gabbros arm/Arm/Mem-Track:** kein paralleles Modell — entweder Master (dann mit Beweis
  nach Lean) oder Spiegel (dann Difftest gegen Lean-Master).
- **Änderungsfluss:** immer zuerst Lean (+Beweis), dann Spiegel + Difftest nachziehen. Nie umgekehrt.
