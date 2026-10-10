# FAILURES — SVE/SME shards (Lane 32)

Shard-stage catalog of `instrs64_sve.sail` + `instrs64_sme.sail`
(`sail-arm/arm-v9.4-a/src/`) as independent shards. KEIN Full-Build:
constructs this stage cannot lower are listed below with exact spans
(`path:line:col-line:col`). Counts are *sites* (whole-word token
occurrences), not definitions.

Method (E3-konform, ohne Build/Testlauf): read-only `grep -w`-Zählung
plus `grep -n`-Spans auf den Originaldateien; Spans per Inspektion
verifiziert. Die Zählungen entsprechen der Tokenizer-Semantik von
`sail-sve-sme` (Kommentar-/String-bereinigt, Ganzwort-Tokens). Exakte
Analyzer-Zahlen liefert der Integrator per
`cargo run --example shard_sve_sme -- <sve> <sme>` (siehe §5).

## 1. Shard parse summary

| file | bytes | lines | `val` | `function` | other top-level |
| --- | ---: | ---: | ---: | ---: | --- |
| `instrs64_sve.sail` | 3521866 | 95548 | 2442 | 3663 | 0 |
| `instrs64_sme.sail` | 1657873 | 42546 | 1110 | 1665 | 0 |
| **total** | **5179739** | **138094** | **3552** | **5328** | **0** |

Beide Shards bestehen ausschließlich aus `val`-Signaturen +
`function`-Definitionen (kein `mapping`/`scattered`/`overload`/
`register`/`type`/`$include` auf Top-Level) — die Shard-Spaltung in
Definitionslisten (`file_start`/`file_end`) ist damit strukturell
unproblematisch. Parse errors: **none** auf Definitionsebene erwartet
(Verifikation durch Integrator, E3-Moratorium).

## 2. Vector-pattern / loop catalog

Ganzwort-Site-Counts (`grep -c -w`, read-only):

| pattern | sve | sme | total |
| --- | ---: | ---: | ---: |
| `foreach_inc` | 1398 | 1340 | 2738 |
| `foreach_dec` | 12 | 0 | 12 |
| `while_loop` | 1 | 0 | 1 |
| `for_loop` (C-style) | 0 | 0 | 0 |
| `elem_read` (`Elem_read`) | 1684 | 1132 | 2816 |
| `elem_set` (`Elem_set`) | 1645 | 537 | 2182 |
| `predicated_exec` (`ActivePredicateElement`) | 743 | 136 | 879 |
| `mask_read` (`P_read`) | 1193 | 242 | 1435 |
| `mask_write` (`P_set`) | 129 | 0 | 129 |
| `vec_reg_read` (`Z_read`) | 1665 | 828 | 2493 |
| `vec_reg_write` (`Z_set`) | 857 | 206 | 1063 |
| `tile_access` (`ZAtile_read` 38 / `ZAtile_set` 39) | 0 | 77 | 77 |
| `vl_dispatch` (`match VL`) | 1240 | 544 | 1784 |
| `constraint_assert` (`assert(constraint(...))`, dominant) | 328¹ | 58¹ | 386¹ |
| `throw` | 1559 | 659 | 2218 |
| `undefined_init` | 1950 | 645 | 2595 |
| `check_enabled` (`Check*`) | 2006 | 712 | 2718 |
| `vl_query` (`CurrentVL_read`) | 1183 | 544 | 1727 |
| `dependent_sig` (`forall` in `val`) | 1221 | 554 | 1775 |

¹ `assert`-Gesamtzeilen; Stichprobe zeigt fast ausschließlich
`assert(constraint(...))`-Form (`instrs64_sve.sail:56`, `:79`, `:82`,
`:129`, `:152`, …). `constraint`-Zeilen gesamt: 249 (sve) / 58 (sme).

`Check*`-Varianten, sve: `CheckSVEEnabled` 993, `CheckSPAlignment` 674,
`CheckNonStreamingSVEEnabled` 263, `CheckStreamingSVEEnabled` 76, Rest
sonstige. SME: `CheckStreamingSVEAndZAEnabled` 346,
`CheckStreamingSVEEnabled` 198, `CheckSPAlignment` 142,
`CheckSMEZT0Enabled` 15, `CheckSMEEnabled` 8, `CheckSMEAndZAEnabled` 3.

`Have*`-Varianten, sve: `HaveSME` 818, `HaveSVE` 769, `HaveSVE2` 277,
`HaveSVE2p1` 232, `HaveSME2` 101, `HaveSME2p1` 42, Rest klein.
SME: `HaveSME2` 418, `HaveSMEI16I64` 68, `HaveSME` 42,
`HaveSMEB16B16` 36, `HaveSME2p1` 27, `HaveSMEF16F16` 20,
`HaveSMEF64F64` 18, `HaveTME` 6 — keine `HaveSVE*`-Guards.

Priorität SVE: `foreach`-Loops (1411) + Element-Accessoren
(`Elem_read`/`Elem_set` 3329) + Prädikation (743 + 1322 Mask-Sites).
Priorität SME: geschachtelte `foreach`-Tiles (1340, z. B. Zeilen 56–57:
zwei Ebenen direkt hintereinander) + `ZAtile_*` (77).

## 3. Fail summary (non-lowerable sites)

| fail kind | sve | sme | total | owning lane |
| --- | ---: | ---: | ---: | --- |
| `loop_lowering` | 1411 | 1340 | 2751 | Lane 27 (loops) |
| `constraint_solving` | ~1549² | ~612² | ~2161² | Lane 04/05 (nexp-constraint/typechecker) |
| `exception_semantics` | ~1887³ | ~717³ | ~2604³ | Lane 28 (early-return-except) |
| `extern_mapping` | ~10447⁴ | ~4423⁴ | ~14870⁴ | Lane 25 (extern-ffi-mapping) |
| `vl_dispatch` | 1240 | 544 | 1784 | Lane 20/21 (mapping-encdec/pattern-exhaustiveness) |
| `undefined_init` | 1950 | 645 | 2595 | Lane 09/22 (mono-anf-lowering/bitvector-semantics) |

² `forall`-Zeilen + `constraint`-Zeilen (Überlappung möglich, daher ~).
³ `throw`-Zeilen + `assert`-Zeilen (nicht-constraint-Anteil aus
Analyzer-Lauf zu bestätigen).
⁴ Summe `Elem_*` + `ActivePredicateElement` + `P_*` + `Z_*` +
`Check*` + `Have*` + `CurrentVL_read` + `ZAtile_*` (exkl. generischer
`CamelCase`-Helper, die nur der Analyzer-Lauf vollständig zählt).

## 4. Fail samples with spans

Alle Spans per `grep -n` verifiziert (Format `path:line:col-line:col`).

### `loop_lowering` → Lane 27 (loops)

Shard `instrs64_sve.sail` (`foreach…in inc`, Standard-Element-Loop):

- `instrs64_sve.sail:53:5-53:11` (`foreach (e from 0 to … in inc)`)
- `instrs64_sve.sail:126:5-126:11`
- `instrs64_sve.sail:199:5-199:11`

Shard `instrs64_sve.sail` (`foreach…in dec`, revers, 12 Sites):

- `instrs64_sve.sail:16093:5-16093:11` (`foreach (e from (elements - 1) to 0 by 1 in dec)`)
- `instrs64_sve.sail:16175:5-16175:11`
- `instrs64_sve.sail:16257:5-16257:11`

Shard `instrs64_sve.sail` (`while`, einzige Site):

- `instrs64_sve.sail:31051:5-31051:9` (`while next < elements & not_bool(ActivePredicateElement(…)) do {` — braucht Lane-27-Lowering + Terminierungsmaß)

Shard `instrs64_sme.sail` (geschachtelte Tile-Loops):

- `instrs64_sme.sail:56:5-56:11` (äußerer `foreach`, `execute`-Tile)
- `instrs64_sme.sail:57:9-57:15` (innerer `foreach`, +4 eingerückt)
- `instrs64_sme.sail:134:5-134:11`

### `constraint_solving` → Lane 04/05 (nexp-constraint/typechecker)

- `instrs64_sve.sail:56:13-56:18` (`assert(constraint('esize in {16, 32, 64}))`)
- `instrs64_sve.sail:79:9-79:14` (dto., weitere Site)
- `instrs64_sve.sail:152:9-152:14` (dto.)

### `exception_semantics` → Lane 28 (early-return-except)

- `instrs64_sve.sail:70:9-70:13` (`throw(Error_Undefined())`)
- `instrs64_sve.sail:73:9-73:13`
- `instrs64_sve.sail:143:9-143:13`

### `extern_mapping` → Lane 25 (extern-ffi-mapping)

- `instrs64_sve.sail:52:26-52:34` (`result : bits('VL) = undefined` — zugleich `undefined_init`, Lane 09/22)
- `instrs64_sme.sail:58:39-58:47` (`Elem_read`)
- `instrs64_sme.sail:54:43-54:53` (`ZAtile_read`)
- `instrs64_sme.sail:46:5-46:33` (`CheckStreamingSVEAndZAEnabled`)

### `vl_dispatch` → Lane 20/21 (mapping-encdec/pattern-exhaustiveness)

- `instrs64_sve.sail:85:5-85:12` (`match VL {`)
- `instrs64_sve.sail:158:5-158:12`
- `instrs64_sme.sail:88:5-88:12`
- `instrs64_sme.sail:166:5-166:12`

### `undefined_init` → Lane 09/22 (mono-anf-lowering/bitvector-semantics)

- `instrs64_sve.sail:52:26-52:34` (`= undefined`)
- `instrs64_sve.sail:125:26-125:34`
- `instrs64_sve.sail:198:26-198:34`

## 5. Regeneration

Exakte Analyzer-Zahlen (Kommentar-/String-bereinigt, mit allen
generischen `CamelCase`-Helpern) erzeugt der Integrator außerhalb des
E3-Moratoriums:

```sh
cargo run --example shard_sve_sme -- <path-to>/instrs64_sve.sail <path-to>/instrs64_sme.sail > FAILURES_SVE_SME.md
```
