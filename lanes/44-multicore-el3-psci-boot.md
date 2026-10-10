# Lane 44 — multicore-el3-psci-boot
Branch: `lane/44-psci-boot` | Status: QUEUED (Phase 2)

## Ziel
EL3/PSCI-Boot: ein Core startet, Rest parkt und wird aufgeweckt.

## Aufgaben
- `reset.sail`-Anbindung (EL3-Einstieg), `PSCI_CPU_ON`-Flow, Secondary-Cores parken (WFE),
  GIC-Wakeup-Pfad als Skizze. `boot/boot.sh`-Kompatibilität im Blick.

## Done
- Boot-Sequenz als Event-Trace-Demo (noch kein Linux).
