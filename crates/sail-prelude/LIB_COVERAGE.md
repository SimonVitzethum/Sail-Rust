# LIB_COVERAGE.md — Lane 81 Korpus-Gate (`sail/lib` → Frontend)

Status: **Treiber als Code (E3)** — kein lokaler Lauf in dieser Lane.
Verifikation auf dem Test-Rig: `cargo test -p sail-prelude`
(Treiber: `crates/sail-prelude/tests/corpus_frontend.rs`,
Modell: `crates/sail-prelude/src/corpus.rs`).

## Gate

Jede Datei läuft durch `sail_parse::bridge_file` (Lane 02 + 63) und
`sail_resolve::resolve` (Lane 03). Ergebnis je Datei:

- `pass` — parse + bridge + resolve fehlerfrei, kein deferred Rest.
- `deferred-gap` — Gate-ok, aber Bridge-Rest (`mini_ast`-Defs, Rohtext im
  Sidecar) mit Owner (Spalte Owner). Kein Fehler, nur Lücke.
- `parse-error` — Owner Lane 02 (`lanes/02-parser.md`).
- `resolve-error` — Owner Lane 03 (`lanes/03-name-resolve.md`).
- `open` — Rig-Lauf offen (Stand dieser Lane, E3).

Scope: **59 `.sail`-Dateien** unter `sail/lib/` (Stand Sail 0.20.3, vgl.
`SAIL_VERSION` in `src/lib.rs`): 26 vendored Top-Level (in
`crates/sail-prelude/sail-lib/`, Test `corpus_vendored_frontend_gate`) +
`core/corelib.sail` + 21 `float/*` + 11 `concurrency_interface/*`
(Test `corpus_full_lib_frontend_gate` mit `SAIL_LIB_DIR`). Andere
Unterverzeichnisse (`hol/`, `isabelle/`, `sv/`, `coverage/`, `nostd/`,
`int128/`) enthalten keine `.sail`-Dateien.

Erwartung aus lesender Analyse (Rig verifiziert): Parse-ok überall;
Resolve-`distincto`-Fehler nirgends (Single-File-Gate, Duplikate erst im
Whole-Program-Lauf sichtbar); deferred-Rest überall dort, wo die Datei
`type`/`struct`/`enum`/`union`/`bitfield` (Lane 30), `register` (Lane 19),
Top-Level-`let` (Lane 07), `scattered`/`clause` (Lane 11),
`outcome` (Lane 08) oder `constraint` (Lane 04) enthält.

## Zusammenfassung

- Gesamt: 59 Dateien (26 vendored + 33 extern).
- Status in dieser Lane: 59× `open` (E3, Lauf auf dem Rig).
- Dateien mit erwartetem deferred-Rest: 19 (Owner je Zeile).
- Dateien ohne erwarteten Rest (reine val/function/mapping/pragma): 40.

## Tabelle

| # | Datei (`sail/lib/`-relativ) | Vendored | Status | Owner bei Lücke | Lücke / Notiz |
| --- | --- | --- | --- | --- | --- |
| 1 | `arith.sail` | ja | open | `lanes/07-rewrites-core.md` | Top-Level-`let` → Bridge-deferred |
| 2 | `asl_compat.sail` | ja | open | `lanes/02-parser.md` | keine Lücke erwartet (val/pragma) |
| 3 | `concurrency_interface.sail` | ja | open | `lanes/29-concurrency-archsem.md` | keine Lücke erwartet (Hülle) |
| 4 | `dec_bits.sail` | ja | open | `lanes/02-parser.md` | keine Lücke erwartet (val/function) |
| 5 | `elf.sail` | ja | open | `lanes/34-elfmain-executable.md` | keine Lücke erwartet (val/pragma) |
| 6 | `exception.sail` | ja | open | `lanes/11-sharding-incremental.md` | `scattered union exception` → deferred (+28) |
| 7 | `exception_basic.sail` | ja | open | `lanes/30-structs-enums-unions.md` | `union exception` → deferred (Type) |
| 8 | `exception_result.sail` | ja | open | `lanes/28-early-return-except.md` | `union clause exception` → deferred (+11) |
| 9 | `float.sail` | ja | open | `lanes/30-structs-enums-unions.md` | `type rounding_mode` → deferred (+24) |
| 10 | `flow.sail` | ja | open | `lanes/30-structs-enums-unions.md` | `type bit` → deferred |
| 11 | `generic_equality.sail` | ja | open | `lanes/02-parser.md` | keine Lücke erwartet (val/overload) |
| 12 | `hex_bits.sail` | ja | open | `lanes/22-bitvector-semantics.md` | keine Lücke erwartet (val/function) |
| 13 | `hex_bits_signed.sail` | ja | open | `lanes/22-bitvector-semantics.md` | keine Lücke erwartet (val/function) |
| 14 | `isla.sail` | ja | open | `lanes/02-parser.md` | keine Lücke erwartet (val/function) |
| 15 | `mapping.sail` | ja | open | `lanes/20-mapping-encdec.md` | Mapdef gebridged, Body im Sidecar |
| 16 | `mono_rewrites.sail` | ja | open | `lanes/07-rewrites-core.md` | keine Lücke erwartet (function/val) |
| 17 | `option.sail` | ja | open | `lanes/30-structs-enums-unions.md` | `union option` → deferred |
| 18 | `prelude.sail` | ja | open | `lanes/11-sharding-incremental.md` | nur `$include`-Pragmas |
| 19 | `real.sail` | ja | open | `lanes/24-real-support.md` | keine Lücke erwartet (val/function) |
| 20 | `result.sail` | ja | open | `lanes/30-structs-enums-unions.md` | `union result` → deferred |
| 21 | `reverse_endianness.sail` | ja | open | `lanes/22-bitvector-semantics.md` | keine Lücke erwartet (val/function) |
| 22 | `smt.sail` | ja | open | `lanes/07-rewrites-core.md` | Top-Level-`let` → deferred |
| 23 | `string.sail` | ja | open | `lanes/02-parser.md` | keine Lücke erwartet (val/overload/pragma) |
| 24 | `vector.sail` | ja | open | `lanes/30-structs-enums-unions.md` | `type bits` → deferred (+22) |
| 25 | `vector_dec.sail` | ja | open | `lanes/11-sharding-incremental.md` | nur `$include <vector.sail>` |
| 26 | `vector_inc.sail` | ja | open | `lanes/11-sharding-incremental.md` | nur `$include <vector.sail>` |
| 27 | `core/corelib.sail` | nein | open | `lanes/02-parser.md` | keine Lücke erwartet (val/function) |
| 28 | `float/add.sail` | nein | open | `lanes/24-real-support.md` | keine Lücke erwartet (float-Ops) |
| 29 | `float/arith_internal.sail` | nein | open | `lanes/24-real-support.md` | keine Lücke erwartet (float-Ops) |
| 30 | `float/classify.sail` | nein | open | `lanes/30-structs-enums-unions.md` | `enum float_class` → deferred (+24) |
| 31 | `float/common.sail` | nein | open | `lanes/19-register-model.md` | type/let/register/struct → deferred (30/07/19) |
| 32 | `float/eq.sail` | nein | open | `lanes/24-real-support.md` | keine Lücke erwartet (float-Ops) |
| 33 | `float/ge.sail` | nein | open | `lanes/24-real-support.md` | keine Lücke erwartet (float-Ops) |
| 34 | `float/ge_quiet.sail` | nein | open | `lanes/24-real-support.md` | keine Lücke erwartet (float-Ops) |
| 35 | `float/gt.sail` | nein | open | `lanes/24-real-support.md` | keine Lücke erwartet (float-Ops) |
| 36 | `float/gt_quiet.sail` | nein | open | `lanes/24-real-support.md` | keine Lücke erwartet (float-Ops) |
| 37 | `float/inf.sail` | nein | open | `lanes/24-real-support.md` | keine Lücke erwartet (float-Ops) |
| 38 | `float/interface.sail` | nein | open | `lanes/24-real-support.md` | keine Lücke erwartet (float-Ops) |
| 39 | `float/le.sail` | nein | open | `lanes/24-real-support.md` | keine Lücke erwartet (float-Ops) |
| 40 | `float/le_quiet.sail` | nein | open | `lanes/24-real-support.md` | keine Lücke erwartet (float-Ops) |
| 41 | `float/lt.sail` | nein | open | `lanes/24-real-support.md` | keine Lücke erwartet (float-Ops) |
| 42 | `float/lt_quiet.sail` | nein | open | `lanes/24-real-support.md` | keine Lücke erwartet (float-Ops) |
| 43 | `float/nan.sail` | nein | open | `lanes/24-real-support.md` | keine Lücke erwartet (float-Ops) |
| 44 | `float/ne.sail` | nein | open | `lanes/24-real-support.md` | keine Lücke erwartet (float-Ops) |
| 45 | `float/normal.sail` | nein | open | `lanes/24-real-support.md` | keine Lücke erwartet (float-Ops) |
| 46 | `float/rounding.sail` | nein | open | `lanes/24-real-support.md` | keine Lücke erwartet (float-Ops) |
| 47 | `float/sign.sail` | nein | open | `lanes/24-real-support.md` | keine Lücke erwartet (float-Ops) |
| 48 | `float/zero.sail` | nein | open | `lanes/24-real-support.md` | keine Lücke erwartet (float-Ops) |
| 49 | `concurrency_interface/barrier.sail` | nein | open | `lanes/08-callgraph-effects.md` | `outcome sail_barrier` → deferred (+29) |
| 50 | `concurrency_interface/cache_op.sail` | nein | open | `lanes/08-callgraph-effects.md` | `outcome sail_cache_op` → deferred (+29) |
| 51 | `concurrency_interface/common.sail` | nein | open | `lanes/29-concurrency-archsem.md` | Deklarationen, Lücke je Konstrukt (30/19/07/08) |
| 52 | `concurrency_interface/emulator_memory.sail` | nein | open | `lanes/29-concurrency-archsem.md` | Deklarationen, Lücke je Konstrukt |
| 53 | `concurrency_interface/exception.sail` | nein | open | `lanes/08-callgraph-effects.md` | mehrere `outcome` → deferred (+29) |
| 54 | `concurrency_interface/read_write.sail` | nein | open | `lanes/29-concurrency-archsem.md` | Deklarationen, Lücke je Konstrukt |
| 55 | `concurrency_interface/read_write_v1.sail` | nein | open | `lanes/30-structs-enums-unions.md` | struct/enum/union/register/let/outcome → deferred (30/19/07/08) |
| 56 | `concurrency_interface/read_write_v2.sail` | nein | open | `lanes/04-nexp-constraint.md` | struct/register/let/outcome/constraint → deferred (+29) |
| 57 | `concurrency_interface/sysreg.sail` | nein | open | `lanes/08-callgraph-effects.md` | `outcome sail_sys_reg_*` → deferred (+29) |
| 58 | `concurrency_interface/tlbi.sail` | nein | open | `lanes/08-callgraph-effects.md` | `outcome sail_tlbi` → deferred (+29) |
| 59 | `concurrency_interface/translation.sail` | nein | open | `lanes/08-callgraph-effects.md` | `outcome sail_translation_*` → deferred (+29) |

## Treiber-Bedienung (Rig)

```sh
# Vendored Gate (26 Dateien, ohne Checkout):
cargo test -p sail-prelude
# Voller Gate (59 Dateien, mit sail-Checkout):
SAIL_LIB_DIR=/pfad/zu/sail/lib cargo test -p sail-prelude
```

Fehler listet der Rig pro Datei (`rel: status at stage: message`),
deferred-Zähler pro Datei im `eprintln`-Log. Inventar-Drift (Dateizahl
≠ 59) lässt den vollen Gate-Test gezielt fehlschlagen.
