# ARM_COVERAGE — arm-v9.4 `src/*.sail` durchs Frontend (Lane 82)

Stand: 10.10.2026 (UTC). Quelle: `sail-arm/arm-v9.4-a/src/*.sail` (nur lesend).
E3-Moratorium: **kein Build, kein Lauf** — alle Angaben per `wc -l` +
`grep -nE`-Inspektion (Top-Level-Köpfe, `throw`/`foreach`/`forall`/
`instantiation`/etc.) plus Abgleich mit den Shard-Berichten der Lanes
31/32/33 und der Lane-45-Fail-Liste. Der Korpus-Treiber
(`crates/sail-arm-corpus`, `#![forbid(unsafe_code)]`) hält dieselbe Tabelle
als Code; Verifikation (Tests) läuft später auf dem Test-Rig.

Status-Begriffe: `ok-shard` = Definitions-Split/Scan ist per Shard-Bericht
belegt (volles Lowering bleibt dort mit Owner-Liste offen); `defizit` =
volles Frontend braucht die genannte Owner-Lane (je genau **ein** Owner).

## Datei-Tabelle (26 Dateien, 464212 Zeilen)

| Datei | Zeilen | Status | Owner | Beleg / Grund |
|---|---|---|---|---|
| `builtins.sail` | 208 | defizit | lane-04 | `forall`/constraints in `val`-Sigs + `foreach`/`throw`/`assert` (Bodies Z. 41ff) |
| `decode_end.sail` | 111 | defizit | lane-21 | 21 `function clause`-Default-Arme mit `throw` (Top-Dispatch `__DecodeA64/A32/T32/T16`) |
| `decode_start.sail` | 89 | defizit | lane-17 | 21 `scattered function`-Deklarationen + `SEE`-Register (Grammatik-Hoheit) |
| `devices.sail` | 286 | defizit | lane-19 | `register __GIC_*` + `let`-Basen + `effect`-Sigs (Register-Modell) |
| `elfmain.sail` | 99 | defizit | lane-34 | C-externe `val`s (`{c: ...}`) + Effekt-Mengen + `main`/`step_model` |
| `event_clauses.sail` | 138 | defizit | lane-38 | `__ListConfig`/`__Reset`/`__EndCycle`-Eventklauseln (Treiber/Config) |
| `fetch.sail` | 389 | defizit | lane-28 | `undefined`-Inits + `throw`-Passthrough + Emulator-Register |
| `impdefs.sail` | 900 | defizit | lane-25 | `ImpDefBool/Int/Bits`-Externs + lange `else`-`if`-Ketten |
| `instrs32.sail` | 48015 | ok-shard | lane-33 | `CORE_REPORT.md`: 4347 Defs (1610 `val`/`function` + 1127 clauses), 1:1:1 Decode→Execute |
| `instrs64.sail` | 53106 | ok-shard | lane-33 | `CORE_REPORT.md`: 3766 Defs (1365 `val`/`function` + 1036 clauses) |
| `instrs64_sme.sail` | 42546 | ok-shard | lane-32 | `FAILURES_SVE_SME.md`: 2775 Köpfe (1110 `val` + 1110 `function` + 555 clauses) |
| `instrs64_sve.sail` | 95548 | ok-shard | lane-32 | `FAILURES_SVE_SME.md`: 6105 Köpfe (2442 `val` + 2442 `function` + 1221 clauses) |
| `interface.sail` | 199 | defizit | lane-29 | 9 `instantiation`s (`sail_mem_*`, `tlbi`, `barrier`, exceptions) + `union`/`struct` |
| `interrupts.sail` | 368 | defizit | lane-22 | reine Funktionsdatei, dichte Bitvektor/Int-Ausdrücke + 1 `throw` |
| `isla_main.sail` | 86 | defizit | lane-08 | `effect {configuration, escape, rmem, …}`-Mengen in `val`-Sigs |
| `isla_splice.sail` | 29 | defizit | lane-38 | Reset/isla-Einstiegspunkte (sonst trivial, Treiber-Verdrahtung) |
| `map_clauses.sail` | 465 | defizit | lane-20 | eine `mapping`-Klausel `__SetConfig` (encdec-Abbildung) |
| `mem.sail` | 232 | defizit | lane-29 | `sail_mem_read/write`-Callgraph + `forall`-Sigs + `match`-Dispatch |
| `no_devices.sail` | 56 | defizit | lane-38 | Device-Stub-Funktionen (Treiber-Seite, sonst trivial) |
| `prelude.sail` | 375 | defizit | lane-18 | `overload`-Operatoren + pure Externs + `type`/`union`/`enum` |
| `reset.sail` | 1466 | defizit | lane-19 | Reset-Schreibung vieler Control-Register (`TakeReset`, `__InitSystem`) |
| `stubs.sail` | 463 | defizit | lane-25 | 105 Stub-Funktionen als Extern-Platzhalter (FFI-Abbildung) |
| `sysregs.sail` | 1481 | defizit | lane-31 | 56 handgeschriebene SysReg-read/set-Funktionen (autogen ist lane-31-ok) |
| `sysregs_autogen.sail` | 160302 | ok-shard | lane-31 | `SHARD_REPORT.md`: 5630 Defs (2815 `val`/`function`-Paare), Peak ~3,0 MiB |
| `termination_measures.sail` | 116 | defizit | lane-26 | 27 `termination_measure`-Deklarationen |
| `v8_base.sail` | 57139 | ok-shard | lane-33 | `CORE_REPORT.md`: 7611 Köpfe (2663 `val`, 2427 `function`, 1386 `register`, 743 `bitfield`, …) |

## Defizit-Liste (20, je 1 Owner) — Arbeitsliste für „ARM in 20GiB"

| # | Datei | Owner | Nächster Schritt (fremde Lane, nicht hier fixieren) |
|---|---|---|---|
| 1 | `builtins.sail` | lane-04 | `forall`/constraint-Sigs + `foreach`/`throw` erst nach Nexp-Constraint |
| 2 | `decode_end.sail` | lane-21 | Default-Arme als priorisierte Match-Kette (kein Sprungtableau) |
| 3 | `decode_start.sail` | lane-17 | `scattered`-Deklarationen in Vollgrammatik |
| 4 | `devices.sail` | lane-19 | GIC/UART-Register + `let`-Inits ins Register-Modell |
| 5 | `elfmain.sail` | lane-34 | C-Externs + `main`/`step_model` als ausführbares Target |
| 6 | `event_clauses.sail` | lane-38 | Eventklauseln in `--sail-libdir`/Config-Verdrahtung |
| 7 | `fetch.sail` | lane-28 | `undefined` + `throw`-Passthrough (Exit/Exception-Semantik) |
| 8 | `impdefs.sail` | lane-25 | `ImpDef*`-Hooks als FFI-Abbildung |
| 9 | `interface.sail` | lane-29 | `instantiation`s + Arch-Instanz (E1: Lean bleibt Master) |
| 10 | `interrupts.sail` | lane-22 | Bitvektor-Ausdruckdruck nach Bitvec-Semantik |
| 11 | `isla_main.sail` | lane-08 | Effekt-Mengen (`effect_label`, Lane 08) |
| 12 | `isla_splice.sail` | lane-38 | Einstiegspunkte in CLI-Treiber |
| 13 | `map_clauses.sail` | lane-20 | Mapping-Klausel realisieren (`realize_mapping`) |
| 14 | `mem.sail` | lane-29 | Speicher-Callgraph + `match`-Dispatch (Archsem) |
| 15 | `no_devices.sail` | lane-38 | Device-Stubs in Treiber |
| 16 | `prelude.sail` | lane-18 | Operator-Overloads + Prelude-Lib |
| 17 | `reset.sail` | lane-19 | Reset-Pfad über Register-Modell |
| 18 | `stubs.sail` | lane-25 | 105 Stubs als FFI-Abbildung |
| 19 | `sysregs.sail` | lane-31 | handgeschriebene SysRegs als Folge-Shard nach Autogen-Muster |
| 20 | `termination_measures.sail` | lane-26 | Maße in Terminierungs-Pass übernehmen |

Genereller Blocker dahinter (kein Extra-Owner je Zeile, siehe
`lanes/45-e2e-tiny-arm-fails.md` F3/F10): Unified-AST (lane-47) +
Diagnostik (lane-35) gelten für alle Bodies; obige Owner sind die jeweils
dominante fachliche Lücke.

## `ok-shard`-Rest (6 Dateien, per Bericht belegt, Lowering-Fails dort verowned)

- lane-33 (`CORE_REPORT.md` §3–§5): `instrs32`/`instrs64`/`v8_base` —
  Execute-Split + Decode-Dispatch + Top-N-Risiken (`FPExpCoefficient_read`,
  MCPY/MSET-`execute_*`); Lean-Druck via Lane 10/11, Maße via Lane 26.
- lane-32 (`FAILURES_SVE_SME.md` §3–§4): `instrs64_sve`/`instrs64_sme` —
  `loop_lowering`→lane-27, `constraint_solving`→lane-04/05,
  `exception_semantics`→lane-28, `extern_mapping`→lane-25,
  `vl_dispatch`→lane-20/21, `undefined_init`→lane-09/22 (mit Spans).
- lane-31 (`SHARD_REPORT.md`): `sysregs_autogen.sail` — Streaming-Split
  (Standard 64 Defs/Block, Peak ~3,0 MiB); Inhalt: reine
  `val`/`function`-Paare.

## Regeneration

Kein Lauf in dieser Lane (E3). Der Integrator verifiziert außerhalb des
Moratoriums (Code liegt, läuft hier nicht):

```sh
cargo test -p sail-arm-corpus
```

Erwartung: 26 Dateien klassifiziert, Summe 464212, 6 ok-shard / 20 Defizite
mit je genau einem Owner.
