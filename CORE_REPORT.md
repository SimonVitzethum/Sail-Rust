# CORE_REPORT — Lane 33: armv9-instrs-core-shard

Quelle: `sail-arm/arm-v9.4-a/src/{instrs32,instrs64,v8_base}.sail`.
Alle Zahlen per Streaming-Scan gemessen
(`cargo run -p sail-instrs-core --example scan -- <datei>`,
`crates/sail-instrs-core/`), kreuzgeprüft mit unabhängiger Zählung.
**Kein Full-Build**: Es wurde nie das Gesamtmodell kompiliert, nur je eine
Datei als Strom gelesen (O(1) Zusatzspeicher relativ zur Dateigröße).

## 1. Dateiübersicht

| Datei | Zeilen | Defs gesamt | `val` | `function` | `function clause` | Sonstige |
|---|---|---|---|---|---|---|
| `instrs32.sail` | 48.015 | 4.347 | 1.610 | 1.610 | 1.127 | 0 |
| `instrs64.sail` | 53.106 | 3.766 | 1.365 | 1.365 | 1.036 | 0 |
| `v8_base.sail` | 57.139 | 7.581 | 2.663 | 2.427 | 0 | 2.491¹ |

¹ `v8_base`: 743 `bitfield` + 1.386 `register` + 145 `type` + 76 `enum` +
139 `let` + 2 `overload` = 2.491 Helfer-Köpfe.

## 2. Shard-Zuordnung (welche Defs wohin)

Regel (`plan::shard_of`): `execute_*` → `exec`, `decode_*` → `decode`,
`function clause` → `dispatch`, Rest → `helper`. `val`-Signaturen erben den
Shard ihrer Funktion über den Namenspräfix.

| Datei | `exec` | `decode` | `dispatch` | `helper` |
|---|---|---|---|---|
| `instrs32.sail` | 966 (= 483× `val`+`function`) | 2.254 (= 1.127×2) | 1.127 | 0 |
| `instrs64.sail` | 658 (= 329×2) | 2.072 (= 1.036×2) | 1.036 | 0 |
| `v8_base.sail` | 0 | 4² | 0 | 7.577 |

² Ausnahme: `decode_max_smeveclen` / `decode_max_sveveclen` (+`val`, @L40133ff)
sind winzige VLEN-Helfer ohne `execute_*`-Call, fallen per Namensregel in den
`decode`-Shard. Bei der Lean-Emission als Helfer behandeln oder Regel um
`decode_max_*` ergänzen.

Externe Shard-Anker (nicht in diesen Dateien, aber Abhängigkeiten):
`scattered function __Decode…`-Deklarationen in `decode_start.sail`,
Default-Arme + Top-Dispatch `__DecodeA64`/`__DecodeA32` in `decode_end.sail`.

## 3. Execute-Split + Decode-Dispatch

Struktur (für beide Instr-Dateien maschinell verifiziert):

- **1 : 1 : 1**: Jede `decode_*`-Funktion ruft **genau eine** `execute_*`-Funktion
  als Tail-Call (instrs32: 1.127/1.127, instrs64: 1.036/1.036 Kanten;
  `decode_multi_execute` leer). Davor nur Feld-Extraktion (`Slice`/`UInt`),
  Feature-Guards (`HaveCSSC()` u. Ä.) und `ConditionPassed()`.
- **n : 1-Sharing**: `execute_*` wird über Enkodierungen geteilt.
  Meistgeteilt (instrs32): `…_VMOV_i_…` (10 Decoder), `…_VLD1_m_…` / `…_VST1_m_…`
  (je 8). Meistgeteilt (instrs64):
  `…_vector_arithmetic_unary_fp16_conv_float_bulk_sisd` (32),
  `…_memory_single_general_immediate_signed_post_idx` (27),
  `…_memory_vector_single_no_wb` / `…_memory_atomicops_ld` (je 24).
- **Dispatch**: Jede `function clause __Decode…` matcht ein 32-Bit-Opcode-Muster
  (`[bitone, …] as __opcode`), trägt einen `SEE < N`-Prioritätswächter
  (`SEE = N` im Rumpf, Erste-passende-Regel — **kein** Sprungtableau) und ruft
  genau eine `decode_*`-Funktion (1.127/1.036 Clause-Kanten, `SEE`-Folgen je
  Ziel streng monoton, keine Verletzung).
- **Lean-Abbildung**: Die gestreuten `__Decode…`-Funktionen werden je **eine**
  Lean-Funktion mit einem Match-Arm pro Clause (Fanout siehe §5);
  jede `execute_*`/`decode_*`-Funktion wird eine Lean-Funktion (§4).

Clause-Ziele je Datei (Fanout-Beitrag dieser Lane):

- `instrs32.sail`: `__DecodeT32` 513, `__DecodeA32_Unconditional` 205,
  `__DecodeA32_DataProMisc` 145, `__DecodeT16` 85, `__DecodeA32_Media` 76,
  `__DecodeA32_SysASIMDFP` 69, `__DecodeA32_BranchBlock` 16,
  `__DecodeA32_LoadStoreImmLit` 10, `__DecodeA32_LoadStoreReg` 8.
- `instrs64.sail`: `__DecodeA64_DataProcFPSIMD` 575, `__DecodeA64_LoadStore` 295,
  `__DecodeA64_DataProcReg` 73, `__DecodeA64_BranchExcSys` 69,
  `__DecodeA64_DataProcImm` 23, `__DecodeA64_Reserved` 1.

## 4. Größte einzelne Lean-Funktionen (Zeilen / `=>`-Arme / `if`s)

`v8_base.sail` stellt alle Schwergewichte:

| Funktion | Datei @Zeile | Zeilen | Arme | `if`s |
|---|---|---|---|---|
| `FPExpCoefficient_read` | v8_base @L30221 | 500 | 163 | 2 |
| `InitFeatureImpl` | v8_base @L901 | 262 | 0 | 0 |
| `AArch32_S1WalkSD` | v8_base @L38721 | 247 | 6 | 20 |
| `BRBEException` | v8_base @L9956 | 223 | 43 | 19 |
| `AArch32_S1WalkLD` | v8_base @L38037 | 217 | 3 | 27 |
| `FPRoundBase__1` | v8_base @L15646 | 207 | 12 | 30 |
| `CountPMUEvents` | v8_base @L4939 | 198 | 23 | 48 |

Größte in den Instr-Dateien:

| Funktion | Datei @Zeile | Zeilen | Arme | `if`s |
|---|---|---|---|---|
| `execute_aarch64_instrs_memory_mcpymset_cpy` | instrs64 @L7840 | 165 | 0 | 24 |
| `execute_aarch64_instrs_memory_mcpymset_setg` | instrs64 @L43343 | 152 | 0 | 21 |
| `decode_msr_imm_aarch64_instrs_system_register_cpsr` | instrs64 @L39059 | 147 | 25 | 18 |
| `execute_aarch64_instrs_memory_mcpymset_cpyf` | instrs64 @L8921 | 139 | 0 | 18 |
| `decode_bti/chkfeat/clrbhb/csdb_aarch64_instrs_system_hints` | instrs64 @L4208ff | je 127 | 32 | 7 |
| `execute_aarch32_instrs_MRS/MSR_br_Op_AS_txt` | instrs32 @L41160/41304 | je 82 | 18 | 4 |
| `execute_aarch32_instrs_DSB_Op_A_txt` | instrs32 @L2971 | 79 | 14 | 5 |
| `decode_aarch32_instrs_VCVT_iv_A1enc_A_txt` u. a. | instrs32 @L22997ff | 69 | 10 | 6 |

## 5. Größte Lean-Funktionen insgesamt (gemergte `__Decode…`, Arme = Clauses)

Je eine Lean-Funktion pro gestreutem Ziel; Arme über alle Dateien summiert
(+1 Default-Arm aus `decode_end.sail`):

| Lean-Funktion | Arme gesamt | davon diese Lane |
|---|---|---|
| `__DecodeA64_SVE` (Lane 32) | 1.161 | 0 |
| `__DecodeA64_SME` (Lane 32) | 617 | 0 |
| `__DecodeA64_DataProcFPSIMD` | 576 | 575 (instrs64) |
| `__DecodeT32` | 514 | 513 (instrs32) |
| `__DecodeA64_LoadStore` | 296 | 295 (instrs64) |
| `__DecodeA32_Unconditional` | 206 | 205 (instrs32) |
| `__DecodeA32_DataProMisc` | 146 | 145 (instrs32) |
| `__DecodeT16` | 86 | 85 (instrs32) |
| `__DecodeA32_Media` | 77 | 76 (instrs32) |
| `__DecodeA64_DataProcReg` | 74 | 73 (instrs64) |
| `__DecodeA64_BranchExcSys` | 70 | 69 (instrs64) |
| `__DecodeA32_SysASIMDFP` | 70 | 69 (instrs32) |
| `__DecodeA64_DataProcImm` | 24 | 23 (instrs64) |

Konsequenz für den Lean-Druck (Lane 10/11): `__Decode…`-Funktionen als
Match-Kette in Clause-Reihenfolge (Priorität!) emittieren, nicht als
Schlüssel-Dispatch; `FPExpCoefficient_read` (163 Arme) und die
MCPY/MSET-`execute_*` (tiefste `if`-Schachtelung) sind die größten
Einzel-Def-Risiken für Terminierungsmaße (Lane 26).

## 6. Lieferumfang dieser Lane

- `crates/sail-instrs-core/`: `scanner` (Streaming-Köpfe + Rumpf-Fold),
  `plan` (Shard-Zuordnung), `dispatch` (Kanten + `SEE`-Monotonie),
  `measure` (Top-N + Fanout), Beispiel `scan`, Fixtures +
  Integrationstest `tests/shard_plan.rs`. `#![forbid(unsafe_code)]`,
  Clippy sauber, `cargo test` grün (12 Unit + 6 Integration).
- `.gitignore`: `target/`, `Cargo.lock` (wird nie committet).
