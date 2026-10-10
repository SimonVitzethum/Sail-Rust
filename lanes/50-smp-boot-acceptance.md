# Lane 50 — smp-boot-acceptance
Branch: `lane/50-smp-boot` | Status: QUEUED (Phase 3, letzte Lane)

## Ziel
SMP-Linux-Boot im extrahierten Emulator als Abnahme (NICHT im Prover interpretiert).

## Aufgaben
- Boot-Harness-Skizze (Kernel-Image, Device-Tree, GIC, Timer, PSCI), Erfolgskriterien
  (Secondary-Cores online, Userspace erreicht), Messung Zeit/RAM.
- Lean-Seite liefert Spec+Beweise, Boot-Nachweis läuft im Rust-Emulator.

## Done
- Abnahmekriterien + Harness-Skizze, kein Shortcut (kein „Boot" ohne Secondary-Cores).
