# Lane 50 — SMP-Boot-Abnahme (Doku + Harness-Skizze)

Status: Skizze, NICHT gelaufen (E3-Test-Moratorium: kein Build/Test durch Lane-Agents).
Boot-Nachweis läuft später zentral durch den Integrator mit Memory-Limits.

## Grundsatz (bindend)

- **Lean = Spec + Beweise.** Speichermodell-/ArchSem-Semantik ändert sich nur in
  Lean (`lean-sail`, `ArmExtras.lean`, generiertes ARM-Modell) mit Beweis (E1).
  Rust ist Spiegel, kein zweites Modell.
- **Boot-Nachweis im extrahierten Emulator.** SMP-Linux-Boot wird NICHT im Prover
  interpretiert (zu langsam, keine Zeit-/RAM-Aussage), sondern im aus
  Lean/Sail extrahierten Emulator (Rust-Binary, vgl. Lane 44/45-Pipeline).
  Lean liefert die Spec + Beweise, die den Boot-Pfad abdecken (PSCI-FSM,
  GIC-Wakeup, Timer, Secondaries-Protokoll); der Emulator liefert den
  ausführbaren Nachweis.
- **Kein Shortcut:** „Boot" ohne Secondary-Cores zählt NICHT (siehe
  Erfolgskriterien). Uniprocessor-Boot ist kein Abnahme-Erfolg.

## Annahmen (Harness-Eingaben)

| # | Komponente | Annahme / Format | Quelle / Bemerkung |
|---|-----------|------------------|--------------------|
| A1 | Kernel-Image | `Image` (raw, little-endian AArch64, `arch/arm64/boot/Image`), Ladeadresse `0x80080000` (Text-Offset-konform) | Integrator stellt Pfad via `--image`; kein vmlinux-ELF nötig |
| A2 | Device-Tree | `.dtb` (FDT), max. 2 MiB, Ladeadresse `0x80000000`; muss `cpus/cpu@N` für N=0..3 (je nach `--smp`), `psci` (`compatible = "arm,psci-1.0"`, `method = "smc"`), `gicv3` (`compatible = "arm,gic-v3"`), `arch_timer` enthalten | `scripts/smp-boot-acceptance.sh --check-dtb` validiert nur Knoten-Namen (kein echter Boot) |
| A3 | GIC | GICv3-Modell im Emulator: Distributor + Redistributor je Core, SGI für Wakeup (Lane-44-GIC-Wakeup-Skizze); Adressen aus DTB (`reg`-Property) | Keine echte Interrupt-Last in der Abnahme — nur Wakeup-Pfad |
| A4 | Timer | `arch_timer` (CNTFRQ aus DTB oder `--freq`), je-Core CNTVCT; Abnahme prüft nur Monotonie + Scheduler-Ticks, keine Timing-Genauigkeit | Kein QEMU-Vergleich nötig |
| A5 | PSCI | `PSCI_CPU_ON` / `CPU_OFF` / `SYSTEM_OFF` via SMC (FID-Tabelle PSCI 1.0); Secondary-Einstieg `spin-table`-kompatibel ODER `psci` je nach DTB (Harness meldet welche) | Semantik aus Lane 44 (`reset.sail`, Secondary parkt in WFE) |
| A6 | UART | PL011 auf fester Adresse aus DTB (`chosen/stdout-path`); Boot-Log wird auf diese UART gematcht | Kein virtio, kein Block-Device |
| A7 | SMP-Geometrie | `--smp N`, N ∈ {2, 4}; Default 2 (OOM-schonend). MPIDR-Affinität 0..N-1 linear | N=1 ist explizit KEIN Abnahme-Lauf (nur Smoke, zählt nicht) |
| A8 | RAM | `--mem 512M` Default; Obergrenze 1 GiB für Abnahme (E3/OOM-Schutz). Keine Swap-Abhängigkeit | Messpunkt M2 prüft Peak-RSS gegen Budget |

## Erfolgskriterien (alle nötig, in Reihenfolge)

1. **K1 — Primär-Core erreicht Kernel-Einstieg:** UART enthält `Booting Linux on physical CPU 0x0` (o. ä. Release-String) binnen `--timeout-boot` (Default 120 s Emulator-Wallclock).
2. **K2 — Secondaries online:** UART/dmesg enthält für jeden Core 1..N-1 `CPU<N>: Booted secondary processor` UND `/sys`-äquivalenter Nachweis (Harness zählt `smp: Brought up N nodes`); `N-1 == 0` → FAIL (Anti-Shortcut-Regel).
3. **K3 — Userspace erreicht:** UART enthält Init-Marker (`--userspace-marker`, Default `Freeing unused kernel image` + `Run /init as init process` ODER Buildroot-`/sbin/init`-Prompt). Reiner Kernel-Panic/oops → FAIL.
4. **K4 — Keine PSCI/GIC-Fehler:** Emulator-Event-Trace enthält keine `psci: DENIED` / `gic: spurious-wakeup` / `secondary: timeout` Events (Lane-44-Trace-Format).
5. **K5 — Determinismus-Smoke:** 2 Wiederholungsläufe (Integrator, nicht Lane-Agent) erreichen K1–K3 mit identischer Core-Zählung (Wallclock darf streuen, siehe Messpunkte).

## Messpunkte Zeit/RAM

| ID | Was | Wie (Integrator, nach E3-Freigabe) | Budget / Ablage |
|----|-----|-------------------------------------|-----------------|
| M1 | Wallclock bis K1 / K2 / K3 | Harness-Timestamps `t_boot`, `t_smp_up`, `t_userspace` aus UART-Match (Sekunden, 1 Nachkomma) | Nur Bericht, kein hartes Limit; Ablage `perf/smp-boot-<date>.md` |
| M2 | Peak-RSS des Emulators | `/usr/bin/time -v` (`Maximum resident set size`) ODER `cgroup peak`; Vergleich gegen `perf/BUDGETS.md` (Lane-14) | > Budget → FAIL, Lane 07/09-Optimierung oder `--smp 2 --mem 512M` |
| M3 | Emulator-Buildgröße | `du -sh target/release/sail-emu` (einmalig) | Nur Bericht |
| M4 | Event-Trace-Größe | Zeilen + Bytes des Lane-44-Traces | > 50 MiB → Trace-Sampling einschalten, kein FAIL |

Lane-Agent misst NICHTS (E3) — alle M-Werte trägt der Integrator nach.

## Harness

- Skizze: `scripts/smp-boot-acceptance.sh` (Skelett, Default `--dry-run`).
- Echte Läufe nur durch Integrator: `./scripts/smp-boot-acceptance.sh --run --image <Image> --dtb <file.dtb> --smp 2`.
- Lane-Agenten rufen NIE `--run` auf (E3/OOM-Schutz); `--check-dtb` ist reines
  Dateiformat-Grep ohne Emulator/Build und ebenfalls nur zur Doku-Kontrolle.

## Done (Lane 50)

- [x] Diese Doku + Harness-Skizze vorhanden, Annahmen K1–K5/M1–M4 festgehalten.
- [ ] (Integrator, nach E3-Freigabe) `--run` mit N=2 UND N=4 je 2× grün, Messwerte in `perf/`.
