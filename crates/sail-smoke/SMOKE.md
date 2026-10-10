# sail-smoke — Smoke-Parität Lean-vs-C

Vergleich (kein C-Backend-Rewrite): 10 kleine Decode/Execute-Fälle aus
`sail/test/lean/*.sail`. Je Fall ist das erwartete Verhalten aus
`.expected.lean` (Lean-Seite) plus der OCaml-C-Semantik (C-Seite) als
dokumentierte Erwartung festgehalten. Die Runner-Skizze (`RunnerConfig`)
vergleicht später beide Binaries; jetzt: Fixture-Tabelle
(`SMOKE_CASES`) + Vergleichslogik auf Log-Schnipseln
(`normalize_log` / `compare_logs`).

## Fälle

| # | ID | Quelle (`sail/test/lean/`) | Funktion | Erwartung Lean (`.expected.lean`) | Erwartung C (OCaml-Semantik) |
|---|----|-----------------------------|----------|-----------------------------------|-------------------------------|
| 1 | `bitvec-eq` | `bitvec_operation.sail` | `bitvector_eq(x, y)` | `(BitVec 16) -> (BitVec 16) -> Bool`; `==` auf BitVec (`bits k_n := BitVec k_n`) | `eq_bits` über `uint16_t`-Limbs; gleiches Bool, keine Trunkierung |
| 2 | `bitvec-add` | `bitvec_operation.sail` | `bitvector_add(x, y)` | `BitVec.add`, trunkiert auf 16 Bit; `0xFFFF + 1 = 0x0000` | `add_bits` mit 16-Bit-Maske; identischer Wrap-around, gleiche Hex-Darstellung |
| 3 | `bitvec-append` | `bitvec_operation.sail` | `bitvector_append(x, y)` | `x +++ y : BitVec 32` (`E_vector_append`), MSB-first | `append_bits(x, y)` konkateniert MSB-first; gleicher 32-Bit-Hex-String |
| 4 | `bitvec-index` | `bitvec_operation.sail` | `bitvector_access'(x, i)` | `BitVec.getMsb`/`getLsb`-Zugriff; `[0]` ist MSB bei `Order dec` | `access_bits` mit Bounds-Check; gleiches Bit, out-of-range ist beidseitig Laufzeitfehler |
| 5 | `bitfield-cr` | `bitfield.sail` | Register `R : cr_type` | `abbrev cr_type := BitVec 8`; Feldzugriff via `extractLsb`; Register `R ↦ BitVec 8` | Masken-Struct mit `get/set_field`-Shifts; gleiche Subranges, gleicher Write-back |
| 6 | `decode-clause-a` | `match_bv.sail` | `decode` Klausel 1 (32-Bit-Pattern mit `Rm`/`Rn`) | `P_vector_concat`-Match zu geschachtelter `BitVec`-Destrukturierung; Guard `Rm == Rn` entscheidet | `match_bits`-Switch über maskierte Konstanten + Gleichheits-Guard; first-match-wins, gleiche Klausel |
| 7 | `decode-guard` | `match_bv.sail` | `write_CSR` (12-Bit mit Guard `unsigned(index) >= 3`) | Pattern-Guard wird zu `if ... then ... else fallthrough` in der Match-Elaboration | `if` nach Masken-Check; Guard-falsch überspringt Klausel, fällt identisch durch |
| 8 | `gpr-vector` | `register_vector.sail` | `rX(n)` / `wX(n, value)` | `GPRs : Vector (RegisterRef … 64) 31`; `rX(31) = 0x0`, `wX(31, _)` No-op | Array aus 31 Slots + PC; Index 31 kurzgeschlossen auf Null-Konstante, gleicher No-op-Store |
| 9 | `range-narrow` | `range.sail` | `f_int(x : range(0,31))` | `Nat`-Arg verfeinert zu `{x // x ≤ 31}`; Ergebnis `0 : {x // -2 ≤ x ∧ x ≤ 2}` | `int` mit Laufzeit-Range-Assertion am Eintritt; gleiches `0`, gleicher Abort bei out-of-range |
| 10 | `loop-sum` | `loop.sail` | `foreach_loopuseindex(m, n)` | `foreach` senkt sich auf fuel-begrenzte Rekursion; `(0,3) = 6` | schlichte `for`-Schleife mit `uint64`-Akku; gleiche Summe 6, leerer Bereich (`m>n`) ergibt 0 |

## Log-Schnipsel (Soll-Logs beider Seiten)

Jeder Fall trägt `lean_log` / `c_log`. Aktuell sind beide Schnipsel je
Fall identisch (nach `normalize_log`), sodass `run_all()` 10/10 Matches
meldet. Repräsentative Beispiele:

- `bitvec-eq`: `bitvector_eq(0x00FF, 0x00FF) = true`
- `bitvec-add`: `bitvector_add(0xFFFF, 0x0001) = 0x0000`
- `bitvec-append`: `bitvector_append(0xABCD, 0x1234) = 0xABCD1234`
- `bitvec-index`: `bitvector_access'(0x8000, 0) = 1`
- `bitfield-cr`: `R.CR0 = 0xA where R = 0xA5`
- `decode-clause-a`: `decode(0b1111100...) clause=1 result=true`
- `decode-guard`: `write_CSR(0xB03) guard=true result=true`
- `gpr-vector`: `rX(31) = 0x0000000000000000; rX(0) after wX = value`
- `range-narrow`: `f_int(31) = 0 in-range`
- `loop-sum`: `foreach_loopuseindex(0, 3) = 6`

## Vergleichslogik

- `normalize_log`: CRLF→LF, Zeilen trimmen, Leerzeilen droppen,
  innere Whitespace-Runs zu einem Space kollabieren.
- `compare_logs(case, lean, c)` → `Comparison::Match` bei normalisierter
  Gleichheit, sonst `Mismatch { case_id, lean_normalized, c_normalized }`.
- `run_all()` führt alle 10 Fixtures gegen ihre gespeicherten Logs aus.

## Runner-Skizze (später: beide Binaries)

`RunnerConfig { lean_bin, c_bin }` baut pro Fall einen `RunnerPlan`
(gleiche Args für beide Binaries). Der künftige Executor läuft
`lean_bin` + `c_bin`, fängt stdout ein und füttert die Schnipsel in
`compare_logs`. Aktuell wird nichts ausgeführt — nur Pläne gebaut
(`plan_for` / `plan_all`, siehe Tests).
