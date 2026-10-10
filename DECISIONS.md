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

## E2 — Nirgends `sorry` im Lean-Modell

- Weder generierter Code noch Templates noch Support-Lib dürfen `sorry` enthalten.
  Lieber harter Fehler als stilles `sorry` (`ArmExtras.lean`: `decreasing_trivial` ohne `sorry`-Fallback).
- CI-Gate (Lane 15): `grep -rn "sorry" --include="*.lean"` über generiertem Projekt muss leer sein.
- Lane 26 schuldet echte Terminierungsmaße statt `sorry`-Fallback; Lane 12 darf nie `sorry` emittieren.

## E3 — Test-Moratorium (OOM-Schutz, kein Swap auf der Maschine)

- Lane-Agents führen KEINE Builds/Tests aus: kein `cargo test/build/check`, kein `lake build`,
  kein `sail`-Lauf. Grund: parallele Toolchain-Läufe über ~40 Worktrees treiben die Maschine
  ohne Swap in den OOM-Killer, der dann den OpenCode-Desktop/Server trifft.
- Verifikation nur per Code-Review + später zentral durch den Integrator mit Memory-Limits.
- Done-Kriterium „Tests grün" ist bis dahin ausgesetzt; ersatzweise: saubere Typen, kleine Commits.
